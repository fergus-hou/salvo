//! csrf middleware

// port from https://github.com/malyn/tide-csrf

pub mod core;

use std::collections::HashSet;
use std::fmt::{self, Formatter};
use std::time::Duration;

use self::core::{
    AesGcmCsrfProtection, CsrfCookie, CsrfProtection, CsrfToken, UnencryptedCsrfCookie, UnencryptedCsrfToken,
};
use cookie::{Cookie, Expiration, SameSite};
use salvo_core::http::headers::HeaderName;
use salvo_core::http::uri::Scheme;
use salvo_core::http::{Method, StatusCode};
use salvo_core::prelude::*;
use salvo_core::Error;

/// key used to save csrf data to depot.
pub const DATA_KEY: &str = "::salvo::extra::csrf::data";

struct CsrfData {
    token: String,
    header_name: HeaderName,
    query_param: String,
    field_name: String,
}
/// Provides access to request-level CSRF values.
pub trait CsrfDepotExt {
    /// Gets the CSRF token for inclusion in an HTTP request header,
    /// a query parameter, or a form field.
    fn csrf_token(&self) -> Option<&str>;

    /// Gets the name of the header in which to returns the CSRF token,
    /// if the CSRF token is being returned in a header.
    fn csrf_header_name(&self) -> Option<&str>;

    /// Gets the name of the query param in which to returns the CSRF
    /// token, if the CSRF token is being returned in a query param.
    fn csrf_query_param(&self) -> Option<&str>;

    /// Gets the name of the form field in which to returns the CSRF
    /// token, if the CSRF token is being returned in a form field.
    fn csrf_field_name(&self) -> Option<&str>;
}

impl CsrfDepotExt for Depot {
    #[inline]
    fn csrf_token(&self) -> Option<&str> {
        self.get::<CsrfData>(DATA_KEY).map(|d| &*d.token)
    }

    #[inline]
    fn csrf_header_name(&self) -> Option<&str> {
        self.get::<CsrfData>(DATA_KEY).map(|d| d.header_name.as_str())
    }

    #[inline]
    fn csrf_query_param(&self) -> Option<&str> {
        self.get::<CsrfData>(DATA_KEY).map(|d| &*d.query_param)
    }

    #[inline]
    fn csrf_field_name(&self) -> Option<&str> {
        self.get::<CsrfData>(DATA_KEY).map(|d| &*d.field_name)
    }
}

/// Cross-Site Request Forgery (CSRF) protection middleware.
pub struct CsrfHandler {
    cookie_path: String,
    cookie_name: String,
    cookie_domain: Option<String>,
    ttl: Duration,
    header_name: HeaderName,
    query_param: String,
    form_field: String,
    protected_methods: HashSet<Method>,
    protect: AesGcmCsrfProtection,
}

impl fmt::Debug for CsrfHandler {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CsrfHandler")
            .field("cookie_path", &self.cookie_path)
            .field("cookie_name", &self.cookie_name)
            .field("cookie_domain", &self.cookie_domain)
            .field("ttl", &self.ttl)
            .field("header_name", &self.header_name)
            .field("query_param", &self.query_param)
            .field("form_field", &self.form_field)
            .field("protected_methods", &self.protected_methods)
            .finish()
    }
}

impl CsrfHandler {
    /// Create a new instance.
    ///
    /// # Defaults
    ///
    /// The defaults for CsrfHandler are:
    /// - cookie path: `/`
    /// - cookie name: `salvo.extra.csrf`
    /// - cookie domain: None
    /// - ttl: 24 hours
    /// - header name: `x-csrf-token`
    /// - query param: `csrf-token`
    /// - form field: `csrf-token`
    /// - protected methods: `[POST, PUT, PATCH, DELETE]`
    #[inline]
    pub fn new(secret: &[u8]) -> Self {
        let mut key = [0u8; 32];
        derive_key(secret, &mut key);

        Self {
            cookie_path: "/".into(),
            cookie_name: "salvo.extra.csrf".into(),
            cookie_domain: None,
            ttl: Duration::from_secs(24 * 60 * 60),
            header_name: HeaderName::from_static("x-csrf-token"),
            query_param: "csrf-token".into(),
            form_field: "csrf-token".into(),
            protected_methods: vec![Method::POST, Method::PUT, Method::PATCH, Method::DELETE]
                .iter()
                .cloned()
                .collect(),
            protect: AesGcmCsrfProtection::from_key(key),
        }
    }

    /// Sets the protection ttl. This will be used for both the cookie
    /// expiry and the time window over which CSRF tokens are considered
    /// valid.
    ///
    /// The default for this value is one day.
    #[inline]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Sets the name of the HTTP header where the middleware will look
    /// for the CSRF token.
    ///
    /// Defaults to "x-csrf-token".
    #[inline]
    pub fn with_header_name(mut self, header_name: HeaderName) -> Self {
        self.header_name = header_name;
        self
    }

    /// Sets the name of the query parameter where the middleware will
    /// look for the CSRF token.
    ///
    /// Defaults to "csrf-token".
    #[inline]
    pub fn with_query_param(mut self, query_param: impl AsRef<str>) -> Self {
        self.query_param = query_param.as_ref().into();
        self
    }

    /// Sets the name of the form field where the middleware will look
    /// for the CSRF token.
    ///
    /// Defaults to "csrf-token".
    #[inline]
    pub fn with_form_field(mut self, form_field: impl AsRef<str>) -> Self {
        self.form_field = form_field.as_ref().into();
        self
    }

    /// Sets the list of methods that will be protected by this
    /// middleware.
    ///
    /// Defaults to `[POST, PUT, PATCH, DELETE]`
    #[inline]
    pub fn with_protected_methods(mut self, methods: &[Method]) -> Self {
        self.protected_methods = methods.iter().cloned().collect();
        self
    }

    #[inline]
    fn build_cookie(&self, secure: bool, cookie_value: String) -> Cookie<'static> {
        let expires = cookie::time::OffsetDateTime::now_utc() + self.ttl;
        let mut cookie = Cookie::build(self.cookie_name.clone(), cookie_value)
            .http_only(true)
            .same_site(SameSite::Strict)
            .path(self.cookie_path.clone())
            .secure(secure)
            .expires(Expiration::DateTime(expires))
            .finish();

        if let Some(cookie_domain) = self.cookie_domain.clone() {
            cookie.set_domain(cookie_domain);
        }

        cookie
    }

    #[inline]
    fn generate_token(&self, existing_cookie: Option<&UnencryptedCsrfCookie>) -> (CsrfToken, CsrfCookie) {
        let existing_cookie_bytes = existing_cookie.and_then(|c| {
            let c = c.value();
            if c.len() < 64 {
                None
            } else {
                let mut buf = [0; 64];
                buf.copy_from_slice(c);
                Some(buf)
            }
        });

        self.protect
            .generate_token_pair(existing_cookie_bytes.as_ref(), self.ttl.as_secs() as i64)
            .expect("couldn't generate token/cookie pair")
    }

    #[inline]
    fn find_csrf_cookie(&self, req: &Request) -> Option<UnencryptedCsrfCookie> {
        req.cookie(&self.cookie_name)
            .and_then(|c| match base64::decode(c.value().as_bytes()) {
                Ok(value) => Some(value),
                Err(e) => {
                    tracing::error!(error = ?e, "base64 decode error");
                    None
                }
            })
            .and_then(|b| match self.protect.parse_cookie(&b) {
                Ok(value) => Some(value),
                Err(e) => {
                    tracing::error!(error = ?e, "parse cookie error");
                    None
                }
            })
    }

    #[inline]
    async fn find_csrf_token(&self, req: &mut Request) -> Result<UnencryptedCsrfToken, Error> {
        // A bit of a strange flow here (with an early exit as well),
        // because we do not want to do the expensive parsing (form,
        // body specifically) if we find a CSRF token in an earlier
        // location. And we can't use `or_else` chaining since the
        // function that searches through the form body is async. Note
        // that if parsing the body fails then we want to returns an
        // StatusError::internal_server_error(), hence the `?`. This is not the same as
        // what we will do later, which is convert failures to *parse* a
        // found CSRF token into Forbidden responses.
        let csrf_token = if let Some(csrf_token) = self.find_csrf_token_in_header(req) {
            csrf_token
        } else if let Some(csrf_token) = self.find_csrf_token_in_query(req) {
            csrf_token
        } else if let Some(csrf_token) = self.find_csrf_token_in_form(req).await {
            csrf_token
        } else {
            return Err(Error::other("not found"));
        };
        self.protect.parse_token(&csrf_token).map_err(Error::other)
    }

    #[inline]
    fn find_csrf_token_in_header(&self, req: &Request) -> Option<Vec<u8>> {
        req.headers()
            .get(&self.header_name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| base64::decode_config(v.as_bytes(), base64::URL_SAFE).ok())
    }

    #[inline]
    fn find_csrf_token_in_query(&self, req: &Request) -> Option<Vec<u8>> {
        req.queries()
            .get(&self.query_param)
            .and_then(|v| base64::decode_config(v.as_bytes(), base64::URL_SAFE).ok())
    }

    #[inline]
    async fn find_csrf_token_in_form(&self, req: &mut Request) -> Option<Vec<u8>> {
        req.form::<String>(&self.form_field)
            .await
            .and_then(|v| base64::decode_config(v.as_bytes(), base64::URL_SAFE).ok())
    }
}

#[salvo_core::async_trait]
impl Handler for CsrfHandler {
    async fn handle(&self, req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
        // We always begin by trying to find the existing CSRF cookie,
        // even if we do not need to protect this method. A new token is
        // generated on every request *based on the encrypted key in the
        // cookie* and so we always want to find the existing cookie in
        // order to generate a token that uses the same underlying key.
        let existing_cookie = self.find_csrf_cookie(req);

        // Is this a protected method? If so, we need to find the token
        // and verify it against the cookie before we can allow the
        // request.
        if self.protected_methods.contains(req.method()) {
            if let Some(cookie) = &existing_cookie {
                if let Ok(token) = self.find_csrf_token(req).await {
                    if self.protect.verify_token_pair(&token, cookie) {
                        tracing::debug!("verified CSRF token");
                    } else {
                        tracing::debug!("rejecting request due to invalid or expired CSRF token");
                        res.set_status_code(StatusCode::FORBIDDEN);
                        ctrl.skip_rest();
                        return;
                    }
                } else {
                    tracing::debug!("rejecting request due to missing CSRF token",);
                    res.set_status_code(StatusCode::FORBIDDEN);
                    ctrl.skip_rest();
                    return;
                }
            } else {
                tracing::debug!("rejecting request due to missing CSRF cookie",);
                res.set_status_code(StatusCode::FORBIDDEN);
                ctrl.skip_rest();
                return;
            }
        }

        // Generate a new cookie and token (using the existing cookie if
        // present).
        let (token, cookie) = self.generate_token(existing_cookie.as_ref());

        // Add the token to the request for use by the application.
        let secure_cookie = req.uri().scheme() == Some(&Scheme::HTTPS);
        depot.insert(
            DATA_KEY,
            CsrfData {
                token: token.b64_url_string(),
                header_name: self.header_name.clone(),
                query_param: self.query_param.clone(),
                field_name: self.form_field.clone(),
            },
        );

        // Add the CSRF cookie to the response.
        let cookie = self.build_cookie(secure_cookie, cookie.b64_string());
        res.add_cookie(cookie);
        ctrl.call_next(req, depot, res).await;
    }
}

#[inline]
fn derive_key(secret: &[u8], key: &mut [u8; 32]) {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(None, secret);
    hk.expand(&[0u8; 0], key)
        .expect("Sha256 should be able to produce a 32 byte key.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use salvo_core::test::{ResponseExt, TestClient};

    const SECRET: [u8; 32] = *b"secrets must be >= 32 bytes long";

    #[handler]
    async fn get_index(depot: &mut Depot) -> String {
        depot.csrf_token().unwrap_or_default().to_owned()
    }
    #[handler]
    async fn post_index() -> &'static str {
        "POST"
    }

    #[tokio::test]
    async fn middleware_exposes_csrf_request_extensions() {
        let router = Router::new().hoop(CsrfHandler::new(&SECRET)).get(get_index);
        let res = TestClient::get("http://127.0.0.1:7979").send(router).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
    }

    #[tokio::test]
    async fn middleware_adds_csrf_cookie_sets_request_token() {
        let router = Router::new().hoop(CsrfHandler::new(&SECRET)).get(get_index);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(router).await;

        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        assert_ne!(res.take_string().await.unwrap(), "");
        assert_ne!(res.cookie("salvo.extra.csrf"), None);
    }

    #[tokio::test]
    async fn middleware_validates_token_in_header() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let csrf_token = res.take_string().await.unwrap();
        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let mut res = TestClient::post("http://127.0.0.1:7979")
            .insert_header("x-csrf-token", csrf_token)
            .insert_header("cookie", cookie.to_string())
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        assert_eq!(res.take_string().await.unwrap(), "POST");
    }

    #[tokio::test]
    async fn middleware_validates_token_in_alternate_header() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET).with_header_name(HeaderName::from_static("x-mycsrf-header")))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let csrf_token = res.take_string().await.unwrap();
        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let mut res = TestClient::post("http://127.0.0.1:7979")
            .insert_header("x-mycsrf-header", csrf_token)
            .insert_header("cookie", cookie.to_string())
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        assert_eq!(res.take_string().await.unwrap(), "POST");
    }

    #[tokio::test]
    async fn middleware_validates_token_in_query() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let csrf_token = res.take_string().await.unwrap();
        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let mut res = TestClient::post(format!("http://127.0.0.1:7979?a=1&csrf-token={}&b=2", csrf_token))
            .insert_header("cookie", cookie.to_string())
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        assert_eq!(res.take_string().await.unwrap(), "POST");
    }
    #[tokio::test]
    async fn middleware_validates_token_in_alternate_query() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET).with_query_param("my-csrf-token"))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let csrf_token = res.take_string().await.unwrap();
        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let mut res = TestClient::post(format!("http://127.0.0.1:7979?a=1&my-csrf-token={}&b=2", csrf_token))
            .insert_header("cookie", cookie.to_string())
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        assert_eq!(res.take_string().await.unwrap(), "POST");
    }

    #[tokio::test]
    async fn middleware_validates_token_in_form() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET).with_query_param("my-csrf-token"))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let csrf_token = res.take_string().await.unwrap();
        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let mut res = TestClient::post("http://127.0.0.1:7979")
            .insert_header("cookie", cookie.to_string())
            .form(&[("a", "1"), ("csrf-token", &*csrf_token), ("b", "2")])
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        assert_eq!(res.take_string().await.unwrap(), "POST");
    }
    #[tokio::test]
    async fn middleware_validates_token_in_alternate_form() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET).with_form_field("my-csrf-token"))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let csrf_token = res.take_string().await.unwrap();
        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);
        let mut res = TestClient::post("http://127.0.0.1:7979")
            .insert_header("cookie", cookie.to_string())
            .form(&[("a", "1"), ("my-csrf-token", &*csrf_token), ("b", "2")])
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        assert_eq!(res.take_string().await.unwrap(), "POST");
    }

    #[tokio::test]
    async fn middleware_rejects_short_token() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let res = TestClient::post("http://127.0.0.1:7979")
            .insert_header("x-csrf-token", "aGVsbG8=")
            .insert_header("cookie", cookie.to_string())
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn middleware_rejects_invalid_base64_token() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);

        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let res = TestClient::post("http://127.0.0.1:7979")
            .insert_header("x-csrf-token", "aGVsbG8")
            .insert_header("cookie", cookie.to_string())
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn middleware_rejects_mismatched_token() {
        let router = Router::new()
            .hoop(CsrfHandler::new(&SECRET))
            .get(get_index)
            .post(post_index);
        let service = Service::new(router);

        let mut res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        let csrf_token = res.take_string().await.unwrap();

        let res = TestClient::get("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::OK);
        let cookie = res.cookie("salvo.extra.csrf").unwrap();

        let res = TestClient::post("http://127.0.0.1:7979").send(&service).await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);

        let res = TestClient::post("http://127.0.0.1:7979")
            .insert_header("x-csrf-token", csrf_token)
            .insert_header("cookie", cookie.to_string())
            .send(&service)
            .await;
        assert_eq!(res.status_code().unwrap(), StatusCode::FORBIDDEN);
    }
}

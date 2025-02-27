//! Logging module
use std::time::Instant;

use tracing::{Instrument, Level};

use salvo_core::async_trait;
use salvo_core::http::{Request, Response, StatusCode};
use salvo_core::routing::FlowCtrl;
use salvo_core::{Depot, Handler};

/// LogHandler
#[derive(Default, Debug)]
pub struct LogHandler;

#[async_trait]
impl Handler for LogHandler {
    async fn handle(&self, req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
        let span = tracing::span!(
            Level::INFO,
            "Request",
            remote_addr = %req.remote_addr().map(|addr|addr.to_string()).unwrap_or_else(|| "[Unknown]".into()),
            version = ?req.version(),
            method = %req.method(),
            path = %req.uri(),
        );

        async move {
            let now = Instant::now();
            ctrl.call_next(req, depot, res).await;
            let duration = now.elapsed();

            let status = match res.status_code() {
                Some(code) => code,
                None => {
                    if res.body().is_none() {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::OK
                    }
                }
            };
            tracing::info!(
                status = %status,
                duration = ?duration,
                "Response"
            );
        }
        .instrument(span)
        .await
    }
}

#[cfg(test)]
mod tests {
    use salvo_core::prelude::*;
    use salvo_core::test::{ResponseExt, TestClient};
    use tracing_test::traced_test;

    use super::*;

    #[tokio::test]
    #[traced_test]
    async fn test_log() {
        #[handler]
        async fn hello() -> &'static str {
            "hello"
        }

        let router = Router::new()
            .hoop(LogHandler)
            .push(Router::with_path("hello").get(hello));

        TestClient::get("http://127.0.0.1:7979/hello")
            .send(router)
            .await
            .take_string()
            .await
            .unwrap();
        assert!(logs_contain("duration"));
    }
}

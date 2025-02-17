use std::io::{Error as IoError, ErrorKind, Result as IoResult};
use std::sync::Arc;
use std::time::Duration;

use rcgen::{Certificate, CertificateParams, CustomExtension, DistinguishedName, PKCS_ECDSA_P256_SHA256};
use tokio_rustls::rustls::sign::{any_ecdsa_type, CertifiedKey};
use tokio_rustls::rustls::PrivateKey;

use super::cache::AcmeCache;
use super::client::AcmeClient;
use super::config::AcmeConfig;
use super::resolver::ResolveServerCert;
use super::{jose, ChallengeType};

pub(crate) async fn issue_cert(
    client: &mut AcmeClient,
    config: &AcmeConfig,
    resolver: &ResolveServerCert,
) -> IoResult<()> {
    tracing::debug!("issue certificate");
    let order_res = client.new_order(&config.domains).await?;
    // trigger challenge
    let mut valid = false;
    for i in 1..5 {
        let mut all_valid = true;
        for auth_url in &order_res.authorizations {
            let res = client.fetch_authorization(auth_url).await?;
            if res.status == "valid" {
                continue;
            }
            all_valid = false;
            if res.status == "pending" {
                let challenge = res.find_challenge(config.challenge_type)?;
                match config.challenge_type {
                    ChallengeType::Http01 => {
                        if let Some(keys) = &config.keys_for_http01 {
                            let key_authorization = jose::key_authorization(&config.key_pair, &challenge.token)?;
                            let mut keys = keys.write();
                            keys.insert(challenge.token.to_string(), key_authorization);
                        }
                    }
                    ChallengeType::TlsAlpn01 => {
                        let key_authorization_sha256 =
                            jose::key_authorization_sha256(&config.key_pair, &challenge.token)?;
                        let auth_key = gen_acme_cert(&res.identifier.value, key_authorization_sha256.as_ref())?;
                        resolver
                            .acme_keys
                            .write()
                            .insert(res.identifier.value.to_string(), Arc::new(auth_key));
                    }
                }
                client
                    .trigger_challenge(&res.identifier.value, config.challenge_type, &challenge.url)
                    .await?;
            } else if res.status == "invalid" {
                tracing::error!(res = ?res, "unable to authorize");
                return Err(IoError::new(
                    ErrorKind::Other,
                    format!(
                        "unable to authorize `{}`: {}",
                        res.identifier.value,
                        res.error.as_ref().map(|problem| &*problem.detail).unwrap_or("unknown")
                    ),
                ));
            }
        }
        if all_valid {
            valid = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(i * 10)).await;
    }
    if !valid {
        return Err(IoError::new(ErrorKind::Other, "authorization failed too many times"));
    }
    // send csr
    let mut params = CertificateParams::new(config.domains.clone());
    params.distinguished_name = DistinguishedName::new();
    params.alg = &PKCS_ECDSA_P256_SHA256;
    let cert = Certificate::from_params(params)
        .map_err(|e| IoError::new(ErrorKind::Other, format!("failed create certificate request: {}", e)))?;
    let pk = any_ecdsa_type(&PrivateKey(cert.serialize_private_key_der())).unwrap();
    let csr = cert
        .serialize_request_der()
        .map_err(|e| IoError::new(ErrorKind::Other, format!("failed to serialize request der {}", e)))?;
    let order_res = client.send_csr(&order_res.finalize, &csr).await?;
    if order_res.status == "invalid" {
        return Err(IoError::new(
            ErrorKind::Other,
            format!(
                "failed to request certificate: {}",
                order_res
                    .error
                    .as_ref()
                    .map(|problem| &*problem.detail)
                    .unwrap_or("unknown")
            ),
        ));
    }
    if order_res.status != "valid" {
        return Err(IoError::new(
            ErrorKind::Other,
            format!(
                "failed to request certificate: unexpected status `{}`",
                order_res.status
            ),
        ));
    }
    // download certificate
    let cert_pem = client
        .obtain_certificate(
            &*order_res
                .certificate
                .as_ref()
                .ok_or_else(|| IoError::new(ErrorKind::Other, "invalid resonse: missing `certificate` url"))?,
        )
        .await?
        .as_ref()
        .to_vec();
    let pkey_pem = cert.serialize_private_key_pem();
    let cert_chain = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .map_err(|e| IoError::new(ErrorKind::Other, format!("invalid pem: {}", e)))?
        .into_iter()
        .map(tokio_rustls::rustls::Certificate)
        .collect();
    let cert_key = CertifiedKey::new(cert_chain, pk);
    *resolver.cert.write() = Some(Arc::new(cert_key));
    tracing::debug!("certificate obtained");
    if let Some(cache_path) = &config.cache_path {
        cache_path
            .write_pkey(&config.directory_name, &config.domains, pkey_pem.as_bytes())
            .await?;
        cache_path
            .write_cert(&config.directory_name, &config.domains, &cert_pem)
            .await?;
    }
    Ok(())
}

#[inline]
fn gen_acme_cert(domain: &str, acme_hash: &[u8]) -> IoResult<CertifiedKey> {
    let mut params = CertificateParams::new(vec![domain.to_string()]);
    params.alg = &PKCS_ECDSA_P256_SHA256;
    params.custom_extensions = vec![CustomExtension::new_acme_identifier(acme_hash)];
    let cert = Certificate::from_params(params)
        .map_err(|_| IoError::new(ErrorKind::Other, "failed to generate acme certificate"))?;
    let key = any_ecdsa_type(&PrivateKey(cert.serialize_private_key_der())).unwrap();
    Ok(CertifiedKey::new(
        vec![tokio_rustls::rustls::Certificate(cert.serialize_der().map_err(
            |_| IoError::new(ErrorKind::Other, "failed to serialize acme certificate"),
        )?)],
        key,
    ))
}

use std::io::Error as IoError;
use std::str::Utf8Error;

use async_trait::async_trait;
use serde::de::value::Error as DeError;
use thiserror::Error;

use crate::http::StatusError;
use crate::{Depot, Request, Response, Writer};

/// Resut type with `ParseError` has it's error type.
pub type ParseResult<T> = Result<T, ParseError>;

/// ParseError, errors happened when read data from http request.
#[derive(Error, Debug)]
pub enum ParseError {
    /// The Hyper request did not have a valid Content-Type header.
    #[error("The Hyper request did not have a valid Content-Type header.")]
    InvalidContentType,

    /// The Hyper request's body is empty.
    #[error("The Hyper request's body is empty.")]
    EmptyBody,

    /// Parse error when parse from str.
    #[error("Parse error when parse from str.")]
    ParseFromStr,

    /// Parse error when parse from str.
    #[error("Parse error when decode url.")]
    UrlDecode,

    /// Deserialize error when parse from request.
    #[error("Deserialize error.")]
    Deserialize(#[from] DeError),

    /// DuplicateKey.
    #[error("DuplicateKey.")]
    DuplicateKey,

    /// The Hyper request Content-Type top-level Mime was not `Multipart`.
    #[error("The Hyper request Content-Type top-level Mime was not `Multipart`.")]
    NotMultipart,

    /// The Hyper request Content-Type sub-level Mime was not `FormData`.
    #[error("The Hyper request Content-Type sub-level Mime was not `FormData`.")]
    NotFormData,

    /// InvalidRange.
    #[error("InvalidRange")]
    InvalidRange,

    /// An multer error.
    #[error("Multer error: {0}")]
    Multer(#[from] multer::Error),

    /// An I/O error.
    #[error("I/O error: {}", _0)]
    Io(#[from] IoError),

    /// An error was returned from hyper.
    #[error("Hyper error: {0}")]
    Hyper(#[from] hyper::Error),

    /// An error occurred during UTF-8 processing.
    #[error("UTF-8 processing error: {0}")]
    Utf8(#[from] Utf8Error),

    /// Serde json error.
    #[error("Serde json error: {0}")]
    SerdeJson(#[from] serde_json::error::Error),
}

#[async_trait]
impl Writer for ParseError {
    #[inline]
    async fn write(mut self, _req: &mut Request, _depot: &mut Depot, res: &mut Response) {
        res.set_status_error(
            StatusError::internal_server_error()
                .with_summary("http read error happened")
                .with_detail("there is no more detailed explanation."),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    #[tokio::test]
    async fn test_write_error() {
        let mut res = Response::default();
        let mut req = Request::default();
        let mut depot = Depot::new();
        let err = ParseError::EmptyBody;
        err.write(&mut req, &mut depot, &mut res).await;
    }
}

//! Mapping use-case failures to HTTP responses.

use axum::http::StatusCode;
use axum::http::header::WWW_AUTHENTICATE;
use axum::response::{IntoResponse, Response};

use crate::application::ServiceError;

/// A use-case failure on its way to becoming an HTTP response.
pub struct HttpError(ServiceError);

impl From<ServiceError> for HttpError {
    fn from(error: ServiceError) -> Self {
        Self(error)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            ServiceError::Unauthenticated => StatusCode::UNAUTHORIZED,
            ServiceError::Forbidden => StatusCode::FORBIDDEN,
            ServiceError::UnknownCache => StatusCode::NOT_FOUND,
            ServiceError::Invalid(_)
            | ServiceError::Nix(_)
            | ServiceError::HashMismatch
            | ServiceError::NarNotUploaded
            | ServiceError::Interrupted => StatusCode::BAD_REQUEST,
            ServiceError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ServiceError::QuotaExceeded => StatusCode::INSUFFICIENT_STORAGE,
            ServiceError::Internal(detail) => {
                // Why: internals go to the log, never to the client.
                tracing::error!(%detail, "request failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response();
            }
        };
        let body = format!("{}\n", self.0);
        if status == StatusCode::UNAUTHORIZED {
            // Why: tells curl-based clients (Nix) to send netrc credentials.
            let challenge = [(WWW_AUTHENTICATE, "Basic realm=\"procurator-cache\"")];
            return (status, challenge, body).into_response();
        }
        (status, body).into_response()
    }
}

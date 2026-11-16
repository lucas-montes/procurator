//! Mapping use-case failures to HTTP responses.

use axum::http::{StatusCode, header};
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
            ServiceError::Invalid(_) | ServiceError::NotFinal(_) => StatusCode::BAD_REQUEST,
            ServiceError::NotFound | ServiceError::CheckNotFound => StatusCode::NOT_FOUND,
            ServiceError::Conflict(_) => StatusCode::CONFLICT,
            ServiceError::Forbidden => StatusCode::FORBIDDEN,
            ServiceError::Unauthenticated => {
                let challenge = [(header::WWW_AUTHENTICATE, "Bearer")];
                return (StatusCode::UNAUTHORIZED, challenge, "invalid credential\n")
                    .into_response();
            }
            ServiceError::Internal(detail) => {
                // Why: internals go to the log, never to the client.
                tracing::error!(%detail, "request failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response();
            }
        };
        (status, format!("{}\n", self.0)).into_response()
    }
}

//! Reading the caller's credential off a request.

use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;

use super::error::HttpError;
use crate::application::ServiceError;
use crate::domain::access::Credential;

/// The request's `Authorization: Bearer …`, if any. Opaque here: the
/// application's `Access` port decides what it means.
///
/// # Errors
///
/// 401 for an `Authorization` header that is not a bearer token.
pub fn credential(headers: &HeaderMap) -> Result<Credential<'_>, HttpError> {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Ok(Credential::None);
    };
    value
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(Credential::Bearer)
        .ok_or(HttpError::from(ServiceError::Unauthenticated))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    use super::credential;
    use crate::domain::access::Credential;

    fn with(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn bearer_tokens_are_read_and_anything_else_is_refused() {
        assert_eq!(credential(&HeaderMap::new()).ok(), Some(Credential::None));
        assert_eq!(
            credential(&with("Bearer pcr_abc")).ok(),
            Some(Credential::Bearer("pcr_abc"))
        );
        assert!(credential(&with("Basic dXNlcjpwYXNz")).is_err());
        assert!(credential(&with("Bearer ")).is_err());
    }
}

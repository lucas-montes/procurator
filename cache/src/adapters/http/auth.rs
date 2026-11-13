//! Authentication: the [`authenticate`] middleware and the [`Caller`] extractor.

use std::borrow::Cow;
use std::future::{Future, ready};
use std::sync::Arc;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use base64::{Engine as _, engine::general_purpose::STANDARD};

use super::error::HttpError;
use crate::application::ports::{BlobStore, Clock, Entropy, Index};
use crate::application::{Service, ServiceError};
use crate::domain::access::Principal;

/// The authenticated caller of a request, placed by [`authenticate`].
pub struct Caller(Principal);

impl Caller {
    /// Who is calling.
    pub fn principal(&self) -> &Principal {
        &self.0
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = HttpError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        // Why: take, not clone — the principal is read once per request.
        let caller = parts
            .extensions
            .remove::<Principal>()
            .map(Self)
            .ok_or_else(|| {
                HttpError::from(ServiceError::Internal(String::from(
                    "route is not behind the authenticate layer",
                )))
            });
        ready(caller)
    }
}

/// Resolve the request's credentials into a [`Principal`] for [`Caller`].
/// No credentials means anonymous; bad credentials stop the request with 401.
pub async fn authenticate<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    mut request: Request,
    next: Next,
) -> Result<Response, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    let principal = {
        let secret = credential(request.headers())?;
        service.authenticate(secret.as_deref()).await?
    };
    request.extensions_mut().insert(principal);
    Ok(next.run(request).await)
}

/// Pull the token secret out of `Authorization`.
/// Accepts `Bearer <token>` and `Basic` with the token as the password,
/// which is what a netrc entry produces.
fn credential(headers: &HeaderMap) -> Result<Option<Cow<'_, str>>, ServiceError> {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Ok(None);
    };
    let (scheme, rest) = value
        .to_str()
        .ok()
        .and_then(|v| v.split_once(' '))
        .ok_or(ServiceError::Unauthenticated)?;
    let rest = rest.trim();
    let secret = if scheme.eq_ignore_ascii_case("bearer") {
        Cow::Borrowed(rest)
    } else if scheme.eq_ignore_ascii_case("basic") {
        let decoded = STANDARD
            .decode(rest)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(ServiceError::Unauthenticated)?;
        let (_user, password) = decoded
            .split_once(':')
            .ok_or(ServiceError::Unauthenticated)?;
        Cow::Owned(String::from(password))
    } else {
        return Err(ServiceError::Unauthenticated);
    };
    if secret.is_empty() {
        return Err(ServiceError::Unauthenticated);
    }
    Ok(Some(secret))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    use super::credential;

    fn secret_of(value: &'static str) -> Result<Option<String>, ()> {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static(value));
        credential(&headers)
            .map(|secret| secret.map(std::borrow::Cow::into_owned))
            .map_err(|_| ())
    }

    #[test]
    fn extracts_bearer_and_basic_secrets() {
        assert!(credential(&HeaderMap::new()).unwrap().is_none());
        assert_eq!(secret_of("Bearer tok"), Ok(Some(String::from("tok"))));
        assert_eq!(secret_of("bearer tok"), Ok(Some(String::from("tok"))));
        // "anyuser:tok"
        assert_eq!(
            secret_of("Basic YW55dXNlcjp0b2s="),
            Ok(Some(String::from("tok")))
        );
        // ":tok" — netrc entries with no login.
        assert_eq!(secret_of("Basic OnRvaw=="), Ok(Some(String::from("tok"))));
    }

    #[test]
    fn malformed_credentials_are_rejected_not_ignored() {
        for bad in [
            "Bearer",
            "Bearer ",
            "Basic !!!",
            "Basic dXNlcg==",
            "Digest x",
            "Basic dXNlcjo=",
        ] {
            assert_eq!(secret_of(bad), Err(()), "{bad:?}");
        }
    }
}

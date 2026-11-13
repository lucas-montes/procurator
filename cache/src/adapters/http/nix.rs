//! Nix binary-cache routes under `/<cache>/`.

use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Router;
use axum::body::{Body, BodyDataStream, Bytes};
use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, Request, State};
use axum::http::StatusCode;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::Stream;
use tokio_util::io::ReaderStream;

use super::auth::Caller;
use super::error::HttpError;
use crate::application::Service;
use crate::application::ports::{BlobStore, Clock, Entropy, Index};

/// Largest `.narinfo` body accepted. Why: real ones are a few hundred bytes.
const MAX_NARINFO_BYTES: usize = 1 << 20;

const NARINFO_TYPE: &str = "text/x-nix-narinfo";
const NAR_TYPE: &str = "application/x-nix-nar";

/// Routes for every Nix cache.
pub fn routes<B, I, E>() -> Router<Arc<Service<B, I, E>>>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    Router::new()
        .route("/{cache}/nix-cache-info", get(cache_info::<B, I, E>))
        .route(
            "/{cache}/{object}",
            get(get_narinfo::<B, I, E>)
                .put(put_narinfo::<B, I, E>)
                .layer(DefaultBodyLimit::max(MAX_NARINFO_BYTES)),
        )
        .route(
            "/{cache}/nar/{file}",
            get(get_nar::<B, I, E>)
                .head(head_nar::<B, I, E>)
                .put(put_nar::<B, I, E>)
                // Why: NARs run to gigabytes; the use case enforces the cap.
                .layer(DefaultBodyLimit::disable()),
        )
}

/// `/{cache}/{hash}.narinfo`. Any other object name is a 404.
struct NarinfoPath {
    cache: String,
    hash: String,
}

impl<S: Send + Sync> FromRequestParts<S> for NarinfoPath {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            let Path((cache, mut object)) =
                Path::<(String, String)>::from_request_parts(parts, state)
                    .await
                    .map_err(IntoResponse::into_response)?;
            let Some(hash_len) = object.strip_suffix(".narinfo").map(str::len) else {
                return Err(StatusCode::NOT_FOUND.into_response());
            };
            // Why: drop the suffix in place instead of allocating the hash.
            object.truncate(hash_len);
            Ok(Self {
                cache,
                hash: object,
            })
        }
    }
}

/// Upload body as the byte stream the use case consumes: the request body
/// with transport errors surfaced as IO errors.
struct UploadStream(BodyDataStream);

impl Stream for UploadStream {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0)
            .poll_next(cx)
            .map(|chunk| chunk.map(|bytes| bytes.map_err(io::Error::other)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

/// A NAR upload: the client's declared length, if any, and the body stream.
struct NarUpload {
    declared_len: Option<u64>,
    stream: UploadStream,
}

impl<S: Send + Sync> FromRequest<S> for NarUpload {
    type Rejection = Infallible;

    fn from_request(
        request: Request,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let declared_len = request
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        ready(Ok(Self {
            declared_len,
            stream: UploadStream(request.into_body().into_data_stream()),
        }))
    }
}

async fn cache_info<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    Path(cache): Path<String>,
    caller: Caller,
) -> Result<String, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    Ok(service.cache_info(caller.principal(), &cache)?.render())
}

async fn get_narinfo<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    NarinfoPath { cache, hash }: NarinfoPath,
    caller: Caller,
) -> Result<Response, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    Ok(
        match service.narinfo(caller.principal(), &cache, &hash).await? {
            Some(body) => ([(CONTENT_TYPE, NARINFO_TYPE)], body).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    )
}

async fn put_narinfo<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    NarinfoPath { cache, hash }: NarinfoPath,
    caller: Caller,
    body: String,
) -> Result<StatusCode, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    service
        .put_narinfo(caller.principal(), &cache, &hash, &body)
        .await?;
    Ok(StatusCode::CREATED)
}

async fn head_nar<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    Path((cache, file)): Path<(String, String)>,
    caller: Caller,
) -> Result<Response, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    Ok(
        match service.nar_size(caller.principal(), &cache, &file).await? {
            Some(size) => ([(CONTENT_TYPE, NAR_TYPE)], [(CONTENT_LENGTH, size)]).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    )
}

async fn get_nar<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    Path((cache, file)): Path<(String, String)>,
    caller: Caller,
) -> Result<Response, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    Ok(
        match service.open_nar(caller.principal(), &cache, &file).await? {
            Some((size, reader)) => (
                [(CONTENT_TYPE, NAR_TYPE)],
                [(CONTENT_LENGTH, size)],
                Body::from_stream(ReaderStream::new(reader)),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    )
}

async fn put_nar<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    Path((cache, file)): Path<(String, String)>,
    caller: Caller,
    upload: NarUpload,
) -> Result<StatusCode, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    service
        .put_nar(
            caller.principal(),
            &cache,
            &file,
            upload.declared_len,
            upload.stream,
        )
        .await?;
    Ok(StatusCode::CREATED)
}

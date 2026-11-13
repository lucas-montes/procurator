//! Token administration API under `/_api/v1/tokens`. Admin tokens only.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use super::auth::Caller;
use super::error::HttpError;
use crate::application::ports::{BlobStore, Clock, Entropy, Index};
use crate::application::{NewToken, Service};
use crate::domain::access::TokenRecord;

/// Routes for managing tokens.
pub fn routes<B, I, E>() -> Router<Arc<Service<B, I, E>>>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    Router::new()
        .route(
            "/_api/v1/tokens",
            get(list::<B, I, E>).post(create::<B, I, E>),
        )
        .route("/_api/v1/tokens/{id}", delete(revoke::<B, I, E>))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    name: String,
    #[serde(default)]
    admin: bool,
    #[serde(default)]
    grants: Vec<GrantBody>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantBody {
    cache: String,
    permission: String,
}

#[derive(Serialize)]
struct TokenBody {
    id: String,
    name: String,
    admin: bool,
    grants: Vec<GrantBody>,
    created_at: i64,
    revoked_at: Option<i64>,
    /// The secret. Present only in the response that creates the token.
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

#[derive(Serialize)]
struct ListBody {
    tokens: Vec<TokenBody>,
}

fn token_body(record: &TokenRecord, secret: Option<&str>) -> TokenBody {
    TokenBody {
        id: String::from(record.id()),
        name: String::from(record.name()),
        admin: record.is_admin(),
        grants: record
            .grants()
            .iter()
            .map(|grant| GrantBody {
                cache: String::from(grant.cache().as_str()),
                permission: String::from(grant.permission().as_str()),
            })
            .collect(),
        created_at: record.created_at(),
        revoked_at: record.revoked_at(),
        token: secret.map(String::from),
    }
}

async fn create<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    caller: Caller,
    Json(request): Json<CreateRequest>,
) -> Result<(StatusCode, Json<TokenBody>), HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    let grants = request
        .grants
        .iter()
        .map(|g| (g.cache.as_str(), g.permission.as_str()))
        .collect();
    let (record, secret) = service
        .create_token(
            caller.principal(),
            NewToken::new(&request.name, request.admin, grants),
        )
        .await?;
    tracing::info!(id = record.id(), name = record.name(), "token created");
    Ok((
        StatusCode::CREATED,
        Json(token_body(&record, Some(secret.as_str()))),
    ))
}

async fn list<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    caller: Caller,
) -> Result<Json<ListBody>, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    let tokens = service.list_tokens(caller.principal()).await?;
    Ok(Json(ListBody {
        tokens: tokens.iter().map(|t| token_body(t, None)).collect(),
    }))
}

async fn revoke<B, I, E>(
    State(service): State<Arc<Service<B, I, E>>>,
    Path(id): Path<String>,
    caller: Caller,
) -> Result<StatusCode, HttpError>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    if service.revoke_token(caller.principal(), &id).await? {
        tracing::info!(id, "token revoked");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

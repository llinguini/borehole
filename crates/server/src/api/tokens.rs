// API token endpoints. These tokens are managed from the dashboard and are
// distinct both from the login JWT and from the CLI shared `tokens` in config.
//
// Storage is in-memory and per dashboard user (keyed by the JWT `sub`/email):
// `email -> Vec<TokenInfo>`. There is no enforcement yet; this is the v2
// management surface. The secret is shown once on creation and never again;
// only its `tnk_xxxxxxxx` prefix (`id`) is listed afterwards.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::auth::AuthClaims;
use crate::api::json_error;
use crate::control::ServerState;

/// Public token view (no secret). The `id` doubles as the displayed prefix.
#[derive(Debug, Clone, Serialize)]
pub struct TokenInfo {
    pub id: String,
    pub name: String,
    pub scope: String,
    /// RFC3339 creation time.
    pub created_at: String,
    pub revoked: bool,
}

/// Body of `POST /api/tokens`.
#[derive(Debug, Deserialize)]
pub struct CreateToken {
    pub name: String,
    pub scope: String,
}

/// `POST /api/tokens` response: the full secret (shown once) plus the stored
/// view. The secret is never returned again.
#[derive(Debug, Serialize)]
pub struct CreatedToken {
    pub token: String,
    #[serde(flatten)]
    pub info: TokenInfo,
}

/// GET /api/tokens — list the authenticated user's tokens.
pub async fn list(auth: AuthClaims, State(state): State<Arc<ServerState>>) -> Json<Vec<TokenInfo>> {
    let store = state.api_tokens.lock().await;
    Json(store.get(&auth.0.sub).cloned().unwrap_or_default())
}

/// POST /api/tokens — mint a token for the authenticated user.
pub async fn create(
    auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateToken>,
) -> Response {
    let scope = match req.scope.as_str() {
        "all" | "tcp" | "http" => req.scope.clone(),
        _ => return (StatusCode::BAD_REQUEST, json_error("invalid scope")).into_response(),
    };
    let name = req.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, json_error("name is required")).into_response();
    }

    // 32 hex chars; the displayed id keeps only the first 8.
    let secret = Uuid::new_v4().simple().to_string();
    let info = TokenInfo {
        id: format!("tnk_{}", &secret[..8]),
        name: name.to_string(),
        scope,
        created_at: Utc::now().to_rfc3339(),
        revoked: false,
    };
    state
        .api_tokens
        .lock()
        .await
        .entry(auth.0.sub)
        .or_default()
        .push(info.clone());

    let full = format!("tnk_{secret}");
    (StatusCode::CREATED, Json(CreatedToken { token: full, info })).into_response()
}

/// DELETE /api/tokens/:id — revoke a token (kept in the list, marked revoked).
pub async fn revoke(
    auth: AuthClaims,
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Response {
    let mut store = state.api_tokens.lock().await;
    let tokens = match store.get_mut(&auth.0.sub) {
        Some(tokens) => tokens,
        None => return (StatusCode::NOT_FOUND, json_error("token not found")).into_response(),
    };
    match tokens.iter_mut().find(|token| token.id == id) {
        Some(token) => {
            token.revoked = true;
            StatusCode::NO_CONTENT.into_response()
        }
        None => (StatusCode::NOT_FOUND, json_error("token not found")).into_response(),
    }
}

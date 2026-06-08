// HTTP/REST API for the dashboard, served by Axum on `bind_http`.
//
// The whole surface lives under `/api`. Authentication is JWT (HS256) signed
// with `config.jwt_secret`; protected handlers take the `auth::AuthClaims`
// extractor, which rejects requests lacking a valid `Authorization: Bearer`.

pub mod auth;
pub mod devices;
pub mod history;
pub mod nodes;
pub mod tokens;
pub mod tunnels;

use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use include_dir::{include_dir, Dir};
use serde_json::json;

/// The compiled dashboard, embedded at build time. The directory is guaranteed
/// to exist by `build.rs` (it may be empty in a dev build that skipped the web
/// build, in which case `/` serves a 404).
static WEB_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../web/build");

/// Builds a uniform `{ "error": "<msg>" }` JSON body for failure responses.
pub(crate) fn json_error(msg: &str) -> Json<serde_json::Value> {
    Json(json!({ "error": msg }))
}

/// Router serving the embedded dashboard. Unknown paths fall back to the SPA
/// entry point so client-side routing works on deep links / reloads.
pub fn static_router() -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/*path", get(serve_static))
}

/// Serves an embedded asset by path, falling back to the SPA index when the
/// path is not a real file (e.g. a client-side route like `/tokens`).
async fn serve_static(Path(path): Path<String>) -> impl IntoResponse {
    match WEB_DIR.get_file(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref().to_string())], file.contents())
                .into_response()
        }
        None => serve_index().await.into_response(),
    }
}

/// Serves the dashboard entry point, or a 404 when it was never built.
async fn serve_index() -> impl IntoResponse {
    match WEB_DIR.get_file("index.html") {
        Some(f) => Html(f.contents_utf8().unwrap_or("")).into_response(),
        None => (StatusCode::NOT_FOUND, "dashboard not built").into_response(),
    }
}

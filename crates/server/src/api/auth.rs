// Authentication endpoints and the JWT extractor used to guard the API.
//
// There is no database yet (v2): users live in `config.users` as
// `{ email, password_hash }` where `password_hash` is the lowercase SHA-256 hex
// digest of the password. Tokens are HS256 JWTs signed with `config.jwt_secret`.

use std::sync::Arc;

use axum::{
    async_trait,
    extract::{FromRequestParts, Query, State},
    http::{header, request::Parts, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::json_error;
use crate::control::ServerState;

/// Lifetime of an issued token.
const TOKEN_TTL_HOURS: i64 = 24;

/// JWT payload. `sub` is the user's email; `exp` is the unix expiry in seconds.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

/// Body of `POST /api/auth/login`.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Successful login response (also used by the CLI redirect flow).
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub jwt: String,
    pub expires_at: String,
}

/// POST /api/auth/login — validate credentials and mint a JWT.
pub async fn login(State(state): State<Arc<ServerState>>, Json(req): Json<LoginRequest>) -> Response {
    if !credentials_valid(&state, &req.email, &req.password) {
        return (StatusCode::UNAUTHORIZED, json_error("invalid credentials")).into_response();
    }
    match issue_token(&state.jwt_secret, &req.email) {
        Ok((jwt, expires_at)) => Json(LoginResponse { jwt, expires_at }).into_response(),
        Err(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, json_error("failed to issue token")).into_response()
        }
    }
}

/// Query string for `GET /api/auth/cli`. The CLI opens this page passing its
/// local `callback`; the form re-submits with `email`/`password` filled in.
#[derive(Debug, Deserialize)]
pub struct CliQuery {
    pub callback: String,
    pub email: Option<String>,
    pub password: Option<String>,
}

/// GET /api/auth/cli — the browser login page used by `borehole login`.
///
/// Without credentials it renders a minimal form. When the form re-submits with
/// valid credentials it redirects to `{callback}?token={jwt}&expires={exp}`,
/// which the CLI's local listener captures.
///
/// NOTE: the form submits via GET (the only method wired for this route), so
/// credentials travel in the URL. This is acceptable for the local dev login
/// flow; a production deployment should switch to a POST form over HTTPS.
pub async fn cli_login_page(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<CliQuery>,
) -> Response {
    if let (Some(email), Some(password)) = (query.email.as_ref(), query.password.as_ref()) {
        if credentials_valid(&state, email, password) {
            if let Ok((jwt, expires_at)) = issue_token(&state.jwt_secret, email) {
                let url = format!(
                    "{}?token={}&expires={}",
                    query.callback,
                    percent_encode(&jwt),
                    percent_encode(&expires_at),
                );
                return Redirect::to(&url).into_response();
            }
        }
        return Html(login_html(&query.callback, true)).into_response();
    }
    Html(login_html(&query.callback, false)).into_response()
}

/// Extractor guarding protected endpoints: requires a valid `Authorization:
/// Bearer <jwt>` header signed with the server's secret.
pub struct AuthClaims(#[allow(dead_code)] pub Claims);

#[async_trait]
impl FromRequestParts<Arc<ServerState>> for AuthClaims {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<ServerState>,
    ) -> Result<Self, Self::Rejection> {
        // Accept the token from `Authorization: Bearer <jwt>` OR a `?token=<jwt>`
        // query param. The query form exists because `EventSource` (the SSE
        // client) cannot set request headers.
        let token = bearer_token(parts)
            .or_else(|| query_token(parts))
            .ok_or_else(|| unauthorized("missing bearer token or token query param"))?;
        let claims = verify_token(&state.jwt_secret, &token)
            .map_err(|_| unauthorized("invalid or expired token"))?;
        Ok(AuthClaims(claims))
    }
}

/// Extracts the JWT from an `Authorization: Bearer <jwt>` header, if present.
fn bearer_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string)
}

/// Extracts the JWT from a `?token=<jwt>` query parameter, if present. JWTs are
/// URL-safe base64, so the raw value needs no percent-decoding.
fn query_token(parts: &Parts) -> Option<String> {
    parts.uri.query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then(|| value.to_string())
    })
}

/// Verifies a JWT against the server secret, returning the decoded claims.
fn verify_token(secret: &str, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

/// Signs a fresh token for `email`, returning `(jwt, expires_at_rfc3339)`.
fn issue_token(secret: &str, email: &str) -> Result<(String, String), jsonwebtoken::errors::Error> {
    let expiry = Utc::now() + Duration::hours(TOKEN_TTL_HOURS);
    let claims = Claims {
        sub: email.to_string(),
        exp: expiry.timestamp() as usize,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;
    Ok((token, expiry.to_rfc3339()))
}

/// Constant-time-ish credential check: the email must match and the SHA-256 of
/// the password must equal the stored hex digest (case-insensitive).
fn credentials_valid(state: &ServerState, email: &str, password: &str) -> bool {
    let hash = sha256_hex(password);
    state
        .users
        .iter()
        .any(|user| user.email == email && user.password_hash.eq_ignore_ascii_case(&hash))
}

/// Lowercase SHA-256 hex digest of `input`.
fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Builds a 401 response with a JSON error body.
fn unauthorized(msg: &str) -> Response {
    (StatusCode::UNAUTHORIZED, json_error(msg)).into_response()
}

/// Percent-encodes a query-string value (RFC 3986 unreserved set kept as-is).
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Minimal HTML escape for attribute values (callback URL).
fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders the minimal browser login form for the CLI flow.
fn login_html(callback: &str, error: bool) -> String {
    let callback = html_escape(callback);
    let banner = if error {
        "<p class=\"err\">Credenciales no válidas.</p>"
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="es">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>borehole — iniciar sesión</title>
<style>
  body {{ background:#0a0b0d; color:#e7e9ee; font-family:ui-monospace,monospace;
          display:flex; min-height:100vh; align-items:center; justify-content:center; }}
  form {{ border:1px solid rgba(232,236,242,.17); background:#0d0f13; padding:28px;
          width:320px; }}
  h1 {{ font-size:14px; letter-spacing:2px; text-transform:uppercase; margin:0 0 18px; }}
  label {{ display:block; font-size:11px; color:#878d97; margin:14px 0 4px; }}
  input {{ width:100%; background:#0a0b0d; border:1px solid rgba(232,236,242,.17);
           color:#e7e9ee; padding:8px; font-family:inherit; }}
  button {{ margin-top:20px; width:100%; background:#5b8def; color:#0a0b0d; border:0;
            padding:10px; font-family:inherit; font-weight:600; cursor:pointer; }}
  .err {{ color:#d76a5c; font-size:12px; margin:0 0 12px; }}
</style>
</head>
<body>
<form method="get" action="/api/auth/cli">
  <h1>borehole</h1>
  {banner}
  <input type="hidden" name="callback" value="{callback}">
  <label for="email">Email</label>
  <input id="email" name="email" type="email" autocomplete="username" required>
  <label for="password">Contraseña</label>
  <input id="password" name="password" type="password" autocomplete="current-password" required>
  <button type="submit">Iniciar sesión</button>
</form>
</body>
</html>"#
    )
}

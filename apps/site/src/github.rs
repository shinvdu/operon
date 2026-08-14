//! GitHub OAuth 2.0 登录。
//!
//! GitHub 的 OAuth App **不是标准 OIDC Provider**（无 `.well-known/openid-configuration`、
//! 不签发 ID Token），因此不走框架的 `OidcRouter`（openidconnect），而是用纯 OAuth 2.0
//! 授权码流程 + userinfo 端点取用户信息：
//!   GET /api/auth/github           → 302 到 GitHub authorize（state 存 cookie）
//!   GET /api/auth/github/callback  → 换 access_token → userinfo → 签发自有 JWT

use std::sync::Arc;

use axum::extract::{Extension, Query};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use operon_core::prelude::*;
use reqwest::Client as HttpClient;
use serde::Deserialize;

pub struct GithubAuth {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    http: HttpClient,
}

impl GithubAuth {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, redirect_uri: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            http: HttpClient::new(),
        }
    }
}

pub fn router(auth: Arc<GithubAuth>) -> Router {
    Router::new()
        .route("/api/auth/github", get(login).layer(Extension(auth.clone())))
        .route("/api/auth/github/callback", get(callback).layer(Extension(auth)))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct GithubUser {
    id: i64,
    login: String,
    email: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
}

/// 发起登录：生成 state → 302 到 GitHub authorize。
async fn login(Extension(auth): Extension<Arc<GithubAuth>>) -> Result<Response, AppError> {
    let state = uuid::Uuid::new_v4().to_string().replace('-', "");
    let authorize_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope=read:user%20user:email&state={}",
        auth.client_id, auth.redirect_uri, state
    );
    let mut response = Redirect::temporary(&authorize_url).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "github_oauth_state={state}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600"
        ))
        .unwrap(),
    );
    Ok(response)
}

/// 回调：验 state → 换 access_token → GitHub userinfo → 签发自有 JWT。
async fn callback(
    Extension(auth): Extension<Arc<GithubAuth>>,
    Extension(state): Extension<AppState>,
    Query(q): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(e) = q.error {
        return Err(AppError::BadRequest(format!("github oauth error: {e}")));
    }
    let code = q.code.ok_or_else(|| AppError::BadRequest("missing code".into()))?;

    // 校验 state（防 CSRF）
    let cookie_state = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .find(|p| p.trim().starts_with("github_oauth_state="))
                .map(|p| p.splitn(2, '=').nth(1).unwrap_or("").trim().to_string())
        })
        .ok_or_else(|| AppError::BadRequest("missing state cookie".into()))?;
    if cookie_state != q.state.unwrap_or_default() {
        return Err(AppError::BadRequest("state mismatch (csrf)".into()));
    }

    // 换 access_token
    let token_resp: TokenResp = auth
        .http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", &auth.client_id),
            ("client_secret", &auth.client_secret),
            ("code", &code),
            ("redirect_uri", &auth.redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .json()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let access_token = token_resp
        .access_token
        .ok_or_else(|| AppError::BadRequest("no access_token".into()))?;

    // userinfo（GitHub 无标准 ID Token，取用户信息）
    let user: GithubUser = auth
        .http
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "operon-site")
        .send()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .json()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // 签发自有 JWT（sub 用 GitHub user id，与第三方解耦）
    let now = operon_core::unix_now();
    let mut extra = serde_json::Map::new();
    extra.insert("login".into(), serde_json::json!(user.login));
    let claims = JwtClaims {
        sub: user.id.to_string(),
        email: user.email.clone(),
        iat: now,
        exp: now + 86400,
        extra,
    };
    let token = state.jwt.sign(&claims)?;
    Ok(Json(serde_json::json!({
        "token": token,
        "sub": user.id,
        "login": user.login,
        "email": user.email,
        "name": user.name,
    }))
    .into_response())
}

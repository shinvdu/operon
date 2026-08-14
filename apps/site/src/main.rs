//! Operon Cloud 公司网站后端（单 Lambda 打天下）。
//!
//! 模块结构（对应文章三层架构）：
//! - `main.rs`    入口：run_with_setup + Router 注册
//! - `models.rs`  模型层：数据结构 + DynamoDB 映射
//! - `handlers.rs` 路由层：认证 + 业务编排
//!
//! 路由：
//! - `GET  /health`                健康检查
//! - `POST /api/leads`             采购需求提交（公开）→ DynamoDB
//! - `POST /api/admin/login`       管理员登录（SSM 密码）→ 签发 JWT
//! - `GET  /api/admin/leads`       需求列表（JWT + admin role 保护）

mod github;
mod handlers;
mod models;

use std::sync::Arc;

use axum::extract::Extension;
use axum::routing::{get, post};
use axum::{Json, Router};
use operon_core::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_with_setup(|state| async move {
        let mut router = Router::new()
            .route("/health", get(|| async {
                Json(serde_json::json!({ "status": "ok", "service": "operon-site" }))
            }))
            .route("/api/leads", post(handlers::submit_lead))
            .route("/api/admin/login", post(handlers::admin_login))
            .route("/api/admin/leads", get(handlers::admin_leads));

        // GitHub OAuth 登录（SSM 配 github_client_id/secret 时启用）
        if let (Some(cid), Some(csec)) = (
            state.config.secret("github_client_id").map(str::to_string),
            state.config.secret("github_client_secret").map(str::to_string),
        ) {
            let base = std::env::var("OPERON_BASE_URL")
                .unwrap_or_else(|_| "https://arch.sky-city.me".into());
            let redirect = format!("{base}/api/auth/github/callback");
            router = router.merge(github::router(Arc::new(github::GithubAuth::new(
                cid, csec, redirect,
            ))));
            tracing::info!("GitHub OAuth 已启用");
        }

        // Google OIDC 登录（SSM 配 google_client_id/secret 时启用；标准 OIDC，走 OidcRouter）
        if let (Some(gcid), Some(gsec)) = (
            state.config.secret("google_client_id").map(str::to_string),
            state.config.secret("google_client_secret").map(str::to_string),
        ) {
            let base = std::env::var("OPERON_BASE_URL")
                .unwrap_or_else(|_| "https://arch.sky-city.me".into());
            let cfg = OidcProviderConfig {
                name: "google".into(),
                issuer_url: "https://accounts.google.com".into(),
                client_id: gcid,
                client_secret: gsec,
                scopes: vec!["openid".into(), "email".into(), "profile".into()],
            };
            // OIDC state cookie key：环境变量 OPERON_OIDC_COOKIE_KEY（base64 32字节）或 dev 默认
            let cookie_key = std::env::var("OPERON_OIDC_COOKIE_KEY")
                .ok()
                .and_then(|b64| {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.decode(b64).ok()
                })
                .and_then(|bytes| bytes.try_into().ok())
                .unwrap_or([7u8; 32]);
            let oidc = OidcRouter::builder()
                .base_url(base)
                .route_prefix("/api/auth")
                .cookie_key(cookie_key)
                .provider(cfg, handlers::SiteOidcHandler)
                .build()
                .await?;
            router = router.merge(oidc.into_router());
            tracing::info!("Google OIDC 已启用");
        }

        // axum 0.8：AppState 用 Extension 注入（serve 只接受 Router<()>）
        Ok(router.layer(Extension(state)).with_operon_defaults())
    })
    .await
}

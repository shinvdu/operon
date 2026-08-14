//! 示例应用：单 Lambda 打天下。
//!
//! 对应文章第三篇架构全景：所有 HTTP 路由塞进同一个 Lambda，
//! axum Router 分发，本地开发与 Lambda 部署共用同一份代码。
//!
//! 路由：
//! - `GET  /health`    健康检查（无需认证）
//! - `POST /users`     创建用户 → DynamoDB
//! - `GET  /users`     列出所有用户（DynamoDB 查询）
//! - `GET  /me`        当前用户（JWT 保护）

use axum::extract::Extension;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use operon_core::prelude::*;
use operon_core::{OidcAuthHandler, OidcProviderConfig, OidcRouter, OidcUserInfo, TokenDelivery};
use operon_dynamo::DynamoClient;
use serde::{Deserialize, Serialize};

/// 数据模型：单表主键 `pk`/`sk`，`pk = "USERS"` 作为集合键。
#[derive(Clone, Serialize, Deserialize)]
struct User {
    pk: String,
    sk: String, // user_id
    user_id: String,
    email: String,
    name: String,
    created_at: u64,
}

#[derive(Deserialize)]
struct CreateUser {
    email: String,
    name: String,
}

#[derive(Serialize)]
struct UserOut {
    user_id: String,
    email: String,
    name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_with_setup(|state| async move {
        let mut router = Router::new()
            .route("/health", get(health))
            .route("/users", get(list_users).post(create_user))
            .route("/me", get(get_me));
        // 可选：挂 OIDC 登录（设 OPERON_OIDC_ISSUER 时启用，用于本地测试）
        if std::env::var("OPERON_OIDC_ISSUER").is_ok() {
            router = router.merge(build_oidc_router().await?);
        }
        // axum 0.8：AppState 用 Extension 注入（serve 只接受 Router<()>）
        Ok(router.layer(Extension(state)).with_operon_defaults())
    })
    .await
}

/// OIDC 演示 handler：认证成功后签发自有 JWT（TokenDelivery::Json）。
struct MyAuthHandler;

#[async_trait::async_trait]
impl OidcAuthHandler for MyAuthHandler {
    async fn on_authenticated(
        &self,
        user_info: OidcUserInfo,
        state: &AppState,
    ) -> Result<(serde_json::Value, TokenDelivery), AppError> {
        let now = operon_core::unix_now();
        let claims = JwtClaims {
            sub: user_info.sub.clone(),
            email: user_info.email.clone(),
            iat: now,
            exp: now + 86400,
            extra: Default::default(),
        };
        let token = state.jwt.sign(&claims)?;
        Ok((
            serde_json::json!({ "token": token, "sub": user_info.sub }),
            TokenDelivery::Json,
        ))
    }
}

/// 构建 OIDC 路由（provider 名固定 mock，配置来自环境变量）。
async fn build_oidc_router() -> anyhow::Result<Router> {
    let cfg = OidcProviderConfig {
        name: "mock".into(),
        issuer_url: std::env::var("OPERON_OIDC_ISSUER")?,
        client_id: std::env::var("OPERON_OIDC_CLIENT_ID").unwrap_or_else(|_| "test-client".into()),
        client_secret: std::env::var("OPERON_OIDC_CLIENT_SECRET")
            .unwrap_or_else(|_| "test-secret".into()),
        scopes: vec!["openid".into(), "email".into(), "profile".into()],
    };
    let base_url = std::env::var("OPERON_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let oidc = OidcRouter::builder()
        .base_url(base_url)
        .cookie_key([7u8; 32]) // dev 固定 cookie key
        .provider(cfg, MyAuthHandler)
        .build()
        .await?;
    Ok(oidc.into_router())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// 表名带环境前缀（dev/prod 隔离），对应文章 4.4 节的自动表前缀。
fn db(state: &AppState) -> DynamoClient {
    DynamoClient::new(
        &state.aws_config,
        format!("{}users", state.config.table_prefix),
    )
}

async fn list_users(
    Extension(state): Extension<AppState>,
) -> Result<Json<Vec<UserOut>>, AppError> {
    let users: Vec<User> = db(&state).query("USERS").await?;
    Ok(Json(
        users
            .into_iter()
            .map(|u| UserOut {
                user_id: u.user_id,
                email: u.email,
                name: u.name,
            })
            .collect(),
    ))
}

async fn create_user(
    Extension(state): Extension<AppState>,
    Json(body): Json<CreateUser>,
) -> Result<(StatusCode, Json<UserOut>), AppError> {
    let user_id = uuid::Uuid::new_v4().to_string();
    let user = User {
        pk: "USERS".into(),
        sk: user_id.clone(),
        user_id: user_id.clone(),
        email: body.email,
        name: body.name,
        created_at: operon_core::unix_now(),
    };
    db(&state).put(&user).await?;
    Ok((
        StatusCode::CREATED,
        Json(UserOut {
            user_id,
            email: user.email,
            name: user.name,
        }),
    ))
}

async fn get_me(JwtAuth(claims): JwtAuth) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "sub": claims.sub,
        "email": claims.email,
        "exp": claims.exp,
    }))
}

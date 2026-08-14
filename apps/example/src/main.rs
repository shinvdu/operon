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

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use operon_core::prelude::*;
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
        let router = Router::new()
            .route("/health", get(health))
            .route("/users", get(list_users).post(create_user))
            .route("/me", get(get_me))
            .with_state(state)
            .with_operon_defaults();
        Ok(router)
    })
    .await
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
    State(state): State<AppState>,
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
    State(state): State<AppState>,
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

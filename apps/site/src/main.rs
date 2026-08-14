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

mod handlers;
mod models;

use axum::routing::{get, post};
use axum::{Json, Router};
use operon_core::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_with_setup(|state| async move {
        let router = Router::new()
            .route("/health", get(|| async {
                Json(serde_json::json!({ "status": "ok", "service": "operon-site" }))
            }))
            .route("/api/leads", post(handlers::submit_lead))
            .route("/api/admin/login", post(handlers::admin_login))
            .route("/api/admin/leads", get(handlers::admin_leads))
            .with_state(state)
            .with_operon_defaults();
        Ok(router)
    })
    .await
}

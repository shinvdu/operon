//! Operon Cloud 公司网站后端（单 Lambda 打天下）。
//!
//! 路由：
//! - `GET  /health`                健康检查
//! - `POST /api/leads`             采购需求提交（公开）→ DynamoDB
//! - `POST /api/admin/login`       管理员登录（SSM 密码）→ 签发 JWT
//! - `GET  /api/admin/leads`       需求列表（JWT + admin role 保护）

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use operon_core::prelude::*;
use operon_dynamo::DynamoClient;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

// ---------- 数据模型 ----------

/// 采购需求（DynamoDB `operon-{env}-leads` 单表：pk="LEADS", sk=零填充时间戳）。
#[derive(Clone, Serialize, Deserialize)]
struct Lead {
    pk: String,
    sk: String,
    id: String,
    name: String,
    company: String,
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    phone: Option<String>,
    requirements: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget: Option<String>,
    status: String, // new / contacted / closed
    created_at: u64,
}

#[derive(Deserialize)]
struct LeadRequest {
    name: String,
    company: String,
    email: String,
    phone: Option<String>,
    requirements: String,
    budget: Option<String>,
}

#[derive(Serialize)]
struct LeadOut {
    id: String,
    name: String,
    company: String,
    email: String,
    phone: Option<String>,
    requirements: String,
    budget: Option<String>,
    status: String,
    created_at: u64,
}

impl From<Lead> for LeadOut {
    fn from(l: Lead) -> Self {
        Self {
            id: l.id,
            name: l.name,
            company: l.company,
            email: l.email,
            phone: l.phone,
            requirements: l.requirements,
            budget: l.budget,
            status: l.status,
            created_at: l.created_at,
        }
    }
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    username: String,
}

// ---------- 入口 ----------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_with_setup(|state| async move {
        let router = Router::new()
            .route("/health", get(health))
            .route("/api/leads", post(submit_lead))
            .route("/api/admin/login", post(admin_login))
            .route("/api/admin/leads", get(admin_leads))
            .with_state(state)
            .with_operon_defaults();
        Ok(router)
    })
    .await
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "operon-site" }))
}

fn leads_table(state: &AppState) -> String {
    format!("{}leads", state.config.table_prefix)
}

// ---------- 采购需求提交 ----------

async fn submit_lead(
    State(state): State<AppState>,
    Json(body): Json<LeadRequest>,
) -> Result<(StatusCode, Json<LeadOut>), AppError> {
    let now = operon_core::unix_now();
    let id = uuid::Uuid::new_v4().to_string();
    let lead = Lead {
        pk: "LEADS".into(),
        sk: format!("{now:020}"), // 零填充时间戳：升序存储，倒序查询
        id: id.clone(),
        name: body.name,
        company: body.company,
        email: body.email,
        phone: body.phone,
        requirements: body.requirements,
        budget: body.budget,
        status: "new".into(),
        created_at: now,
    };
    let db = DynamoClient::new(&state.aws_config, leads_table(&state));
    db.put(&lead).await?;
    tracing::info!(lead_id = %id, "new procurement lead submitted");
    Ok((StatusCode::CREATED, Json(lead.into())))
}

// ---------- 管理员登录 ----------

async fn admin_login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    if body.username != "admin" {
        return Err(AppError::Unauthorized("invalid credentials".into()));
    }
    // 密码存 SSM（SecureString），冷启动批量拉取；dev 模式从环境变量注入
    let expected = state
        .config
        .secret("admin_password")
        .map(str::to_string)
        .or_else(|| std::env::var("OPERON_DEV_ADMIN_PASSWORD").ok())
        .ok_or_else(|| AppError::Internal("admin_password not configured".into()))?;
    let ok = expected.as_bytes().ct_eq(body.password.as_bytes());
    if !bool::from(ok) {
        return Err(AppError::Unauthorized("invalid credentials".into()));
    }

    let now = operon_core::unix_now();
    let mut extra = serde_json::Map::new();
    extra.insert("role".into(), serde_json::json!("admin"));
    let claims = JwtClaims {
        sub: "admin".into(),
        email: None,
        iat: now,
        exp: now + 8 * 3600, // 8 小时会话
        extra,
    };
    let token = state.jwt.sign(&claims)?;
    Ok(Json(LoginResponse {
        token,
        username: "admin".into(),
    }))
}

// ---------- 管理员查看需求 ----------

async fn admin_leads(
    State(state): State<AppState>,
    JwtAuth(claims): JwtAuth,
) -> Result<Json<Vec<LeadOut>>, AppError> {
    let is_admin = claims
        .extra
        .get("role")
        .and_then(|v| v.as_str())
        .map(|r| r == "admin")
        .unwrap_or(false);
    if !is_admin {
        return Err(AppError::Forbidden("admin only".into()));
    }
    let db = DynamoClient::new(&state.aws_config, leads_table(&state));
    let mut leads: Vec<Lead> = db.query("LEADS").await?;
    leads.reverse(); // sk 升序存储 → 倒序 = 最新在前
    Ok(Json(leads.into_iter().map(LeadOut::from).collect()))
}

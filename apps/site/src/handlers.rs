//! 路由处理器：认证 + 业务逻辑编排。
//! 对应文章三层架构的「路由层」：极薄，只做提取认证、解析参数、调数据层。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use operon_core::prelude::*;
use operon_dynamo::DynamoClient;
use subtle::ConstantTimeEq;

use crate::models::*;

/// 表名带环境前缀（dev/prod 隔离）。
fn leads_table(state: &AppState) -> String {
    format!("{}leads", state.config.table_prefix)
}

/// POST /api/leads —— 采购需求提交（公开）→ DynamoDB。
pub async fn submit_lead(
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

/// POST /api/admin/login —— 管理员登录（SSM 密码，常量时间比对）→ 签发 JWT。
pub async fn admin_login(
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

/// GET /api/admin/leads —— 需求列表（JWT + admin role 保护，时间倒序）。
pub async fn admin_leads(
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

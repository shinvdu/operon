//! 路由处理器：认证 + 业务逻辑编排。
//! 对应文章三层架构的「路由层」：极薄，只做提取认证、解析参数、调数据层。

use axum::extract::Extension;
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

/// Google OIDC 登录回调：认证成功后签发自有 JWT（TokenDelivery::Json）。
pub struct SiteOidcHandler;

#[async_trait::async_trait]
impl OidcAuthHandler for SiteOidcHandler {
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

/// POST /api/leads —— 采购需求提交（公开）→ DynamoDB。
pub async fn submit_lead(
    Extension(state): Extension<AppState>,
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
    Extension(state): Extension<AppState>,
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
    Extension(state): Extension<AppState>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use operon_core::{AppConfig, Jwt, JwtClaims};
    use std::collections::HashMap;
    use tower::ServiceExt;

    const DEV_PASSWORD: &str = "correct-pass";

    async fn test_state() -> AppState {
        std::env::set_var("OPERON_DEV_ADMIN_PASSWORD", DEV_PASSWORD);
        // 显式 region 避免 IMDS 探测延迟
        let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region("us-west-2")
            .load()
            .await;
        AppState {
            config: AppConfig {
                project: "test".into(),
                environment: "test".into(),
                region: "us-west-2".into(),
                table_prefix: "test-".into(),
                secrets: HashMap::new(),
                dev_mode: true,
            },
            jwt: Jwt::from_seed(&[7u8; 32]).unwrap(),
            aws_config,
        }
    }

    async fn app() -> Router {
        Router::new()
            .route("/api/admin/login", axum::routing::post(admin_login))
            .route("/api/admin/leads", axum::routing::get(admin_leads))
            .layer(Extension(test_state().await))
    }

    async fn post_json(uri: &str, body: &str) -> axum::response::Response {
        app()
            .await
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn login_wrong_password_401() {
        let res = post_json("/api/admin/login", r#"{"username":"admin","password":"wrong"}"#).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_wrong_username_401() {
        let res =
            post_json("/api/admin/login", r#"{"username":"other","password":"correct-pass"}"#).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_correct_returns_token() {
        let res =
            post_json("/api/admin/login", r#"{"username":"admin","password":"correct-pass"}"#).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["token"].is_string());
        assert_eq!(json["username"], "admin");
    }

    #[tokio::test]
    async fn leads_without_token_401() {
        let res = app()
            .await
            .oneshot(Request::builder().uri("/api/admin/leads").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn leads_non_admin_403() {
        // 普通用户 token（无 role=admin）
        let jwt = Jwt::from_seed(&[7u8; 32]).unwrap();
        let now = operon_core::unix_now();
        let claims = JwtClaims {
            sub: "user".into(),
            email: None,
            iat: now,
            exp: now + 3600,
            extra: Default::default(),
        };
        let token = jwt.sign(&claims).unwrap();
        let res = app()
            .await
            .oneshot(
                Request::builder()
                    .uri("/api/admin/leads")
                    .header("X-Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
}

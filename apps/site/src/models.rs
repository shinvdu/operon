//! 数据模型层：DynamoDB 单表映射 + API 请求/响应结构。
//! 对应文章三层架构的「模型层」：专注数据结构与序列化，不含业务逻辑。

use serde::{Deserialize, Serialize};

/// 采购需求（DynamoDB `operon-{env}-leads` 单表：pk="LEADS", sk=零填充时间戳）。
#[derive(Clone, Serialize, Deserialize)]
pub struct Lead {
    pub pk: String,
    pub sk: String,
    pub id: String,
    pub name: String,
    pub company: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    pub requirements: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<String>,
    /// 状态：new / contacted / closed
    pub status: String,
    pub created_at: u64,
}

/// 采购需求提交请求体（前端表单 POST /api/leads）。
#[derive(Deserialize)]
pub struct LeadRequest {
    pub name: String,
    pub company: String,
    pub email: String,
    pub phone: Option<String>,
    pub requirements: String,
    pub budget: Option<String>,
}

/// 采购需求输出（对外不暴露 pk/sk）。
#[derive(Serialize)]
pub struct LeadOut {
    pub id: String,
    pub name: String,
    pub company: String,
    pub email: String,
    pub phone: Option<String>,
    pub requirements: String,
    pub budget: Option<String>,
    pub status: String,
    pub created_at: u64,
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

/// 管理员登录请求体。
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 登录响应（签发 JWT）。
#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
}

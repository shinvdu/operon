//! 数据模型层：DynamoDB 单表映射 + API 请求/响应结构。
//! 对应文章三层架构的「模型层」：专注数据结构与序列化，不含业务逻辑。

use serde::{Deserialize, Serialize};

/// 采购需求（DynamoDB `operon-{env}-leads` 单表：pk="LEADS", sk=零填充时间戳）。
/// 登录用户提交时关联 `user_id`，并双写 GSI1（gsi1pk=`USER#{sub}`）支持「我的记录」查询。
#[derive(Clone, Serialize, Deserialize)]
pub struct Lead {
    pub pk: String,
    pub sk: String,
    pub id: String,
    /// 提交用户的 JWT sub（登录时才有）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// GSI1 分区键：`USER#{sub}`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gsi1pk: Option<String>,
    /// GSI1 排序键：零填充时间戳
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gsi1sk: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_lead() -> Lead {
        Lead {
            pk: "LEADS".into(),
            sk: "00000000000000000123".into(),
            id: "id-1".into(),
            user_id: Some("user-1".into()),
            gsi1pk: Some("USER#user-1".into()),
            gsi1sk: Some("00000000000000000123".into()),
            name: "张伟".into(),
            company: "示例科技".into(),
            email: "a@b.com".into(),
            phone: Some("13800000000".into()),
            requirements: "需要一个官网".into(),
            budget: Some("5k-20k".into()),
            status: "new".into(),
            created_at: 123,
        }
    }

    #[test]
    fn lead_to_out_excludes_pk_sk() {
        let out = LeadOut::from(sample_lead());
        assert_eq!(out.id, "id-1");
        assert_eq!(out.name, "张伟");
        let json = serde_json::to_value(&out).unwrap();
        assert!(json.get("pk").is_none(), "对外不应暴露 pk");
        assert!(json.get("sk").is_none(), "对外不应暴露 sk");
    }

    #[test]
    fn lead_serialization_roundtrip() {
        let json = serde_json::to_value(&sample_lead()).unwrap();
        assert_eq!(json["pk"], "LEADS");
        assert_eq!(json["status"], "new");
        assert_eq!(json["sk"], "00000000000000000123");
    }

    #[test]
    fn lead_request_deserializes_optional_fields() {
        let req: LeadRequest =
            serde_json::from_str(r#"{"name":"李","company":"C","email":"x@y.z","requirements":"r"}"#)
                .unwrap();
        assert_eq!(req.name, "李");
        assert!(req.phone.is_none());
        assert!(req.budget.is_none());
    }
}

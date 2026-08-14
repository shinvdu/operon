//! Webhook 签名验证（对应文章 4.5 节「可插拔架构」）。
//!
//! - `WebhookVerifier` trait：任何第三方回调都可实现；
//! - `HmacVerifier`：通用 HMAC-SHA256，支持 Stripe / GitHub 的签名前缀；
//! - 使用方式：作为 axum 中间件，在请求体到达业务 handler 前完成验签。

use axum::http::HeaderMap;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use serde::de::DeserializeOwned;
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    #[error("malformed body: {0}")]
    BadBody(String),
}

/// 验证器 trait：从 headers + body 验签并解析事件。
#[async_trait::async_trait]
pub trait WebhookVerifier: Send + Sync + 'static {
    type Event: DeserializeOwned + Send;
    async fn verify(&self, headers: &HeaderMap, body: &Bytes) -> Result<Self::Event, WebhookError>;
}

/// 通用 HMAC-SHA256 验证器。
///
/// `signature_prefix` 用于剥离提供方签名里的前缀（Stripe 是 `v1=`，GitHub 是 `sha256=`），
/// 传 `Some("v1=")` 即可复用为 Stripe 验证器，`Some("sha256=")` 即 GitHub 验证器。
pub struct HmacVerifier {
    secret: String,
    header_name: String,
    signature_prefix: Option<String>,
}

impl HmacVerifier {
    pub fn new(secret: impl Into<String>, header_name: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            header_name: header_name.into(),
            signature_prefix: None,
        }
    }

    /// 设置签名前缀（如 `v1=`、`sha256=`）。
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.signature_prefix = Some(prefix.into());
        self
    }

    pub fn stripe(secret: impl Into<String>) -> Self {
        Self::new(secret, "x-stripe-signature").with_prefix("v1=")
    }

    pub fn github(secret: impl Into<String>) -> Self {
        Self::new(secret, "x-hub-signature-256").with_prefix("sha256=")
    }

    fn compute(&self, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes()).expect("hmac key");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }
}

#[async_trait::async_trait]
impl WebhookVerifier for HmacVerifier {
    type Event = serde_json::Value;

    async fn verify(&self, headers: &HeaderMap, body: &Bytes) -> Result<Self::Event, WebhookError> {
        let provided = headers
            .get(&self.header_name)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| WebhookError::InvalidSignature(format!("missing {} header", self.header_name)))?;

        // 剥离前缀（如 "sha256=abc..." → "abc..."）
        let provided = match &self.signature_prefix {
            Some(p) => provided
                .strip_prefix(p.as_str())
                .ok_or_else(|| WebhookError::InvalidSignature(format!("signature missing prefix '{p}'")))?,
            None => provided,
        };

        let expected = self.compute(body);
        if !bool::from(expected.as_bytes().ct_eq(provided.as_bytes())) {
            return Err(WebhookError::InvalidSignature("signature mismatch".into()));
        }
        serde_json::from_slice(body).map_err(|e| WebhookError::BadBody(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[tokio::test]
    async fn hmac_valid() {
        let body = Bytes::from(r#"{"event":"payment","id":"evt_1"}"#);
        let sig = sign("sec-123", &body);
        let v = HmacVerifier::new("sec-123", "x-signature");
        let mut h = HeaderMap::new();
        h.insert("x-signature", HeaderValue::from_str(&sig).unwrap());
        let evt = v.verify(&h, &body).await.unwrap();
        assert_eq!(evt["event"], "payment");
    }

    #[tokio::test]
    async fn hmac_invalid_signature_rejected() {
        let body = Bytes::from(r#"{"event":"payment"}"#);
        let v = HmacVerifier::new("sec-123", "x-signature");
        let mut h = HeaderMap::new();
        h.insert("x-signature", HeaderValue::from_str("deadbeef").unwrap());
        assert!(matches!(v.verify(&h, &body).await, Err(WebhookError::InvalidSignature(_))));
    }

    #[tokio::test]
    async fn hmac_missing_header_rejected() {
        let body = Bytes::from(r#"{"event":"payment"}"#);
        let v = HmacVerifier::new("sec-123", "x-signature");
        let h = HeaderMap::new();
        assert!(matches!(v.verify(&h, &body).await, Err(WebhookError::InvalidSignature(_))));
    }

    #[tokio::test]
    async fn github_prefix_stripped() {
        let body = Bytes::from(r#"{"action":"opened"}"#);
        let sig = format!("sha256={}", sign("github-sec", &body));
        let v = HmacVerifier::github("github-sec");
        let mut h = HeaderMap::new();
        h.insert("x-hub-signature-256", HeaderValue::from_str(&sig).unwrap());
        let evt = v.verify(&h, &body).await.unwrap();
        assert_eq!(evt["action"], "opened");
    }

    #[tokio::test]
    async fn stripe_prefix_required() {
        let body = Bytes::from(r#"{"type":"invoice.paid"}"#);
        let sig = format!("v1={}", sign("stripe-sec", &body));
        let v = HmacVerifier::stripe("stripe-sec");
        let mut h = HeaderMap::new();
        h.insert("x-stripe-signature", HeaderValue::from_str(&sig).unwrap());
        let evt = v.verify(&h, &body).await.unwrap();
        assert_eq!(evt["type"], "invoice.paid");
    }
}

# Operon OIDC 登录（OpenID Connect）

> 对应文章 4.3 节「第三层：OIDC 登录流程」。基于 **openidconnect**（通过 OpenID
> Relying Party Certification 官方一致性认证）实现，本地 E2E 已验证通过（2026-08-15）。

---

## 一、实现概述

| 组件 | 说明 |
|---|---|
| **协议处理** | openidconnect 库：Discovery、Authorization Code + PKCE、ID Token 验证（JWKS） |
| **state 存储** | AES-256-GCM 加密进 cookie（含 csrf / nonce / pkce_verifier），**无服务端存储** |
| **框架 API** | `OidcRouter::builder()`（对齐文章原文） |
| **业务接入** | 实现 `OidcAuthHandler`，认证成功后签发自有 JWT |
| **代码位置** | `crates/core/src/oidc.rs` |

## 二、登录流程

```
用户点击登录
  └─ GET /api/auth/{provider}
       ├─ openidconnect 生成 PKCE(code_challenge) + csrf + nonce
       ├─ state(csrf+nonce+verifier) → AES-256-GCM 加密 → HttpOnly cookie
       └─ 302 → IdP authorize 端点
用户授权后
  └─ GET /api/auth/{provider}/callback?code=..&state=..
       ├─ 解密 cookie，校验 csrf（防 CSRF）
       ├─ 换 token（Authorization Code + PKCE verifier）
       ├─ openidconnect 验证 ID Token（RS256 签名 / aud / iss / exp / nonce）
       └─ 解析 OidcUserInfo → 调用业务 OidcAuthHandler → 签发自有 JWT
另外
  └─ GET /.well-known/jwks.json  → 框架 Ed25519 公钥（供第三方验证框架签发的 JWT）
```

## 三、框架 API 用法

```rust
use operon_core::prelude::*;
use operon_core::{OidcAuthHandler, OidcProviderConfig, OidcRouter, OidcUserInfo, TokenDelivery};

// 1. 实现业务回调
struct MyAuthHandler;
#[async_trait::async_trait]
impl OidcAuthHandler for MyAuthHandler {
    async fn on_authenticated(
        &self, user_info: OidcUserInfo, state: &AppState,
    ) -> Result<(serde_json::Value, TokenDelivery), AppError> {
        // 查找/创建用户（用 user_info.sub / email）
        let claims = JwtClaims { sub: user_info.sub.clone(), email: user_info.email.clone(), .. };
        let token = state.jwt.sign(&claims)?;
        // TokenDelivery::Cookie：写 HttpOnly cookie 后跳转
        Ok((serde_json::json!({"token": token}), TokenDelivery::Json))
    }
}

// 2. 构建 OIDC 路由并挂载
async fn build_oidc() -> anyhow::Result<Router> {
    let cfg = OidcProviderConfig {
        name: "google".into(),
        issuer_url: "https://accounts.google.com".into(),
        client_id: std::env::var("GOOGLE_CLIENT_ID")?.into(),
        client_secret: std::env::var("GOOGLE_CLIENT_SECRET")?.into(),
        scopes: vec!["openid".into(), "email".into(), "profile".into()],
    };
    let oidc = OidcRouter::builder()
        .base_url("https://your-domain.example.com")
        .route_prefix("/api/auth")
        .cookie_key(cookie_key_bytes)   // 32 字节
        .provider(cfg, MyAuthHandler)
        .build().await?;                 // build 时做 .well-known Discovery
    Ok(oidc.into_router())
}

// 3. main() 里 merge
run_with_setup(|state| async move {
    let router = Router::new()
        .route("/health", get(health))
        .merge(build_oidc().await?)
        .layer(Extension(state))          // axum 0.8 用 Extension 注入 AppState
        .with_operon_defaults();
    Ok(router)
})
```

### TokenDelivery 两种交付
- **`Cookie { name, max_age_secs, path, redirect_url }`**：签发 JWT 写 HttpOnly cookie 后 302 跳转
- **`Json`**：直接把业务 handler 返回的 JSON 返回给客户端

---

## 四、本地测试（mock IdP，无需真实账号）

仓库自带 `infra/mock_idp.py`（模拟 OIDC 服务器，签发 RS256 ID Token）：

```bash
# 1. 启动 mock IdP（127.0.0.1:9090）
python3 infra/mock_idp.py &

# 2. 启动带 OIDC 的应用（example 演示）——设 OPERON_OIDC_ISSUER 启用
export OPERON_DEV_JWT_SEED="$(cargo run -q -p operon-cli -- dev-seed)"
export OPERON_OIDC_ISSUER="http://127.0.0.1:9090"
export OPERON_BASE_URL="http://127.0.0.1:8080" PORT=8080
cargo run -p operon-example &

# 3. 走完整流程
BASE=http://127.0.0.1:8080
curl -s -D h1.txt -o /dev/null -c ck.txt "$BASE/api/auth/mock"          # → 307 + Set-Cookie
LOC=$(grep -i '^location:' h1.txt | tr -d '\r' | cut -d' ' -f2)
curl -s -D h2.txt -o /dev/null -b ck.txt -c ck.txt "$LOC"               # mock authorize → 302 callback
CALLBACK=$(grep -i '^location:' h2.txt | tr -d '\r' | cut -d' ' -f2)
curl -s -b ck.txt "$CALLBACK"                                           # → JSON 含自有 JWT
```

---

## 五、后期配置真实 IdP

### Google
1. [Google Cloud Console](https://console.cloud.google.com) → 项目 → **APIs & Services → OAuth consent screen**（External，填应用名、支持邮箱）
2. **Credentials → Create Credentials → OAuth client ID → Web application**
3. **Authorized redirect URIs** 加：`https://你的域名/api/auth/google/callback`
4. 记下 **Client ID** 和 **Client Secret**
5. 配置到应用（环境变量或 SSM），`issuer_url = https://accounts.google.com`

### GitHub（已实装，走 OAuth 2.0）
> GitHub 的 OAuth App **不是标准 OIDC Provider**（无 ID Token），本仓库已实装为纯
> **OAuth 2.0 授权码流程**（`apps/site/src/github.rs`）：state cookie → authorize →
> 换 access_token → GitHub userinfo → 签自有 JWT。线上验证通过。

1. GitHub → **Settings → Developer settings → OAuth Apps → New OAuth App**
2. **Homepage URL** 填：`https://你的域名`
3. **Authorization callback URL** 填：`https://你的域名/api/auth/github/callback`
   - ⚠️ **OAuth App 没有 Webhook URL**——那是 GitHub App 的字段（收 GitHub 事件用），与登录无关，别用 GitHub App
4. 记下 **Client ID** / **Client Secret**
5. 存 SSM（SecureString）：
   ```bash
   aws ssm put-parameter --name /operon/dev/github_client_id --type SecureString --value <Client ID> --overwrite
   aws ssm put-parameter --name /operon/dev/github_client_secret --type SecureString --value <Client Secret> --overwrite
   ```
6. 部署后 `GET /api/auth/github` 发起登录，回调返回自有 JWT（sub = GitHub user id）

### 部署
- `client_id/secret` 存 SSM（SecureString），冷启动经 `AppConfig.secret()` 读取，明文不入仓库
- `cookie_key`（32 字节）：SSM 生成，多实例保持一致

---

## 六、安全说明

- **PKCE**：S256 挑战，防授权码拦截
- **state 加密**：AES-256-GCM，篡改即拒绝（CSRF 防护）
- **nonce**：绑定到登录会话，防重放
- **ID Token 验证**：openidconnect 校验 RS256 签名（JWKS）+ aud + iss + exp + nonce
- **cookie**：`HttpOnly; SameSite=Lax`，JS 不可读
- **自有 JWT**：业务用框架 `Jwt`（Ed25519）签发，经 `/jwks.json` 可被第三方验证

---

## 七、排坑记录

1. **openidconnect 要求 token 响应含 `access_token`**（缺则 "Failed to parse server response"）
2. **ID Token claims 不能有 `null` 值字段**（如 `picture: null` → 反序列化报错）
3. **openidconnect 用 Basic auth 发 client 凭证**（不在 body），mock 需固定 client_id
4. **mock 重启换 RSA key** → 应用缓存的 JWKS 过期，需同时重启应用
5. **axum 0.8 serve 只接受 `Router<()>`** → 应用用 `Extension(state)` 注入 AppState，`OidcRouter::into_router()` 返回 `Router`

---

## 八、相关代码

- **OIDC（标准 Provider）**：`crates/core/src/oidc.rs`（openidconnect，Google 等）
- **GitHub OAuth 2.0 适配**：`apps/site/src/github.rs`（授权码 + userinfo，已线上验证）
- 演示：`apps/example/src/main.rs`（`MyAuthHandler` + `build_oidc_router`）
- 测试：`infra/mock_idp.py`（本地模拟 IdP）
- 状态：
  - OIDC（openidconnect）：**已上线 + 线上验证**（Google，xie.tianbao44@gmail.com 登录成功）
  - GitHub OAuth：**已上线 + 线上验证**（shinvdu 登录成功，JWT 校验通过）
  - 部署：SSM 配 `google_client_id/secret` + `github_client_id/secret` 时启用（SecureString，明文不入仓库）

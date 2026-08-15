---
name: operon
description: 使用 operon 无服务器框架（Rust + Lambda + axum）开发、构建、测试、部署应用时使用。提供框架结构、认证、CLI、部署约定和排坑。
---

# operon 框架技能

operon 是复现陈天《一人公司》系列技术方案的无服务器框架：**单 Lambda + axum + Web Adapter**，
AppState 用 `Extension` 注入（axum 0.8 的 serve 只接受 `Router<()>`）。

## 何时使用
- 用户要搭建/修改一个 operon 应用（新 API、路由、认证、模型）
- 用户提到：operon、单 Lambda、Function URL、OIDC 登录、DynamoDB 单表

## 项目结构
```
crates/
├── core/      # 运行时(run_with_setup)、中间件、JWT、API Key、OIDC、SQS、配置(SSM)
├── dynamo/    # DynamoDB 薄封装（get/put/query/query_index，表前缀，错误映射）
├── webhook/   # WebhookVerifier trait + HmacVerifier（Stripe/GitHub）
└── s3/        # S3Client（put/get + presign_get/presign_put）
apps/<app>/
└── src/
    ├── main.rs     # 入口：run_with_setup + Router 注册
    ├── models.rs   # 模型层：DynamoDB 映射 + API 请求/响应
    └── handlers.rs # 路由层：handler + 认证
frontend/      # 原生 HTML/JS（index.html / my.html / admin.html）
infra/         # CloudFormation 模板 + deploy.sh + params.json
```

## 常用命令
```bash
# 构建（必须 musl 静态链接，兼容 Lambda AL2023）
cargo build --release --target x86_64-unknown-linux-musl -p <app>

# 本地开发（无需 AWS）
export OPERON_DEV_JWT_SEED="$(cargo run -q -p operon-cli -- dev-seed)" PORT=8080
cargo run -p <app>

# CLI 一条龙
cargo run -q -p operon-cli -- init --template api my-app   # 脚手架
cargo run -q -p operon-cli -- deploy --env dev            # 部署
cargo run -q -p operon-cli -- logs --env dev              # CloudWatch 日志
cargo run -q -p operon-cli -- gen-seed                    # 生成密钥
cargo run -q -p operon-cli -- token --seed <b64> --sub <id>  # 签 JWT

# 部署
cd infra && ./deploy.sh dev
```

## 新增路由三步
1. `main.rs` 的 `Router::new()` 注册：`.route("/path", get(handlers::xxx))`
2. `handlers.rs` 加 handler：签名 `Extension(state): Extension<AppState>`（取框架注入状态）+ 返回 `Result<Json<T>, AppError>`
3. `models.rs` 加请求/响应结构（serde `Serialize`/`Deserialize`）

### handler 约定
```rust
pub async fn list_users(
    Extension(state): Extension<AppState>,     // 取 AppState（config/jwt/aws_config）
    JwtAuth(claims): JwtAuth,                  // JWT 保护（X-Authorization: Bearer）
    Json(body): Json<CreateUser>,              // 请求体
) -> Result<(StatusCode, Json<Vec<User>>), AppError> { ... }
```
- 错误统一用 `AppError`（自动转 `{"error":{"code","message"}}`）
- DynamoDB 错误 `?` 自动映射（dynamo crate）

## 数据模型（DynamoDB 单表）
- 单表主键 `pk`/`sk`；表名自动带环境前缀（`operon-{env}-leads`）
- 登录用户关联：`user_id` + GSI1（`gsi1pk=USER#{sub}`, `gsi1sk=时间戳`），查询用 `db.query_index("gsi1", ...)`
- 时间排序：`sk = 零填充时间戳`（升序存储，读后 reverse 为最新在前）

## 认证体系
| 方式 | 用法 | 头/参数 |
|---|---|---|
| JWT（Ed25519） | handler 参数 `JwtAuth(claims): JwtAuth` | `X-Authorization: Bearer <jwt>` |
| API Key | handler 参数 `ApiKeyAuth` | `X-API-Key`（SSM 配 `api_key`） |
| OIDC（Google） | `OidcRouter::builder()...provider(cfg, handler).build().await` | SSM 配 `google_client_id/secret` |
| GitHub OAuth | `github.rs`（OAuth 2.0 授权码 + userinfo） | SSM 配 `github_client_id/secret` |

- **JWT 必须走 `X-Authorization`**（CloudFront OAC 占用标准 `Authorization` 头）
- OIDC state 用 AES-256-GCM 加密 cookie；登录成功回调返回 HTML 存 token 到 localStorage 并跳 `/my.html`

## 测试约定（铁律）
- 新功能/修改**必须加单元测试**，`cargo test` 全绿才提交
- 纯逻辑：模块内 `#[cfg(test)] mod tests`
- handler：`tower::ServiceExt::oneshot` 构造 HTTP 请求断言状态码（用 `Extension(state)` 注入测试 AppState）
- 依赖 AWS 的调用本地不跑；只测不依赖云的部分

## 部署与运维
- `./infra/deploy.sh dev`：编译 musl → 打包 bootstrap.zip → 上传 → CloudFormation（Lambda/URL/DynamoDB/S3/CloudFront）
- **走代理** `source ~/proxy.sh`；凭证从项目根 `.env`（gitignore 排除，不入库）
- 静态参数在 `infra/params.json`；`ACM_ARN` 等可环境变量覆盖
- 证书自动续期：`acme.sh` cron → `infra/renew-acm.sh`
- 成本告警：AWS Budgets（$10/月）→ SNS → 邮件

## 排坑速查
1. **musl 静态编译**：本机 glibc > Lambda AL2023，必须 `--target x86_64-unknown-linux-musl`
2. **OAC 双权限**：Lambda 需要 `lambda:InvokeFunctionUrl` + `lambda:InvokeFunction`
3. **S3 OAC 要 bucket policy**（模板已含 `FrontendBucketPolicy`）
4. **POST 必须带 `x-amz-content-sha256`**（Lambda Function URL 不支持 unsigned payload），前端用 `crypto.subtle`
5. **axum 0.8 serve 只接受 `Router<()>`**：AppState 用 `Extension(state)` 注入，handlers 用 `Extension<AppState>`
6. **自定义头转发**：CloudFront 需 OriginRequestPolicy（AllViewerExceptHostHeader）
7. **NameSilo 限制**：CNAME 拒下划线值、无 NS → 用 Let's Encrypt TXT 绕开

## 参考
- `README.md` / `CLAUDE.md` / `docs/`（REQUIREMENTS/DESIGN/API/OIDC/AWS-OPERATIONS/FINAL-REPORT）

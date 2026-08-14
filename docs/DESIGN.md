# operon 技术设计文档（TDD）

> **定位**：这份文档是 operon 的「施工图纸」，供 Claude Code 按图施工、供人审阅关键路径。
> **依据**：`REQUIREMENTS.md`（需求）、陈天《一人公司》第三/五篇、当前骨架实现。
> **标记**：`[已实现]` = 骨架已落地；`[待实现]` = 设计定稿待编码。
> **原则**：薄封装、少抽象、为 AI 协作设计、Rust 类型系统兜底质量。

---

## 1. 架构总览

### 1.1 分层与依赖方向

```
┌─────────────────────────────────────────────────────────────┐
│  apps/           业务应用（example, slides…）                  │
│   └── 只依赖 operon-core / operon-dynamo / 共享 crate          │
├─────────────────────────────────────────────────────────────┤
│  operon-core     运行时·中间件·认证·配置·错误    ← 依赖核心     │
│  operon-dynamo   数据访问薄封装                                 │
│  operon-s3       对象存储 + 预签名 URL                          │
│  operon-webhook  回调签名验证                                   │
├─────────────────────────────────────────────────────────────┤
│  operon-facade   统一门面 prelude（按需引入，控制 API 表面积）    │
├─────────────────────────────────────────────────────────────┤
│  operon-cli      脚手架·构建·部署·日志                          │
│  infra/          CloudFormation 模板 + deploy.sh               │
└─────────────────────────────────────────────────────────────┘
```

- **依赖方向单向**：app → facade → {core, dynamo, s3, webhook}；core 不依赖其他框架 crate。
- workspace 根 `Cargo.toml` 集中 `[workspace.dependencies]`，统一版本（第五篇观点）。

### 1.2 运行时拓扑

```
本地:   cargo run ──► axum :8080 （同一份代码）
Lambda: CloudFront ─OAC SigV4─► Function URL(AWS_IAM) ─► Web Adapter 层
        ─► 转发到本地端口 8080 ─► axum Router
```

关键点：**Lambda 里跑的二进制和本地跑的是同一个**。Web Adapter 作为 Lambda 扩展把
`AWS Lambda Function URL` 的 HTTP 请求转发进应用监听端口，应用无需感知 Lambda 事件格式。

---

## 2. 核心模块设计

### 2.1 operon-core

#### 2.1.1 运行时引导 `[已实现]`

```rust
// server.rs —— 一个进程一条主流程
pub async fn run_with_setup<F, Fut>(setup: F) -> anyhow::Result<()>
where
    F: FnOnce(AppState) -> Fut,
    Fut: Future<Output = anyhow::Result<Router>>,
{
    init_tracing();                                  // Lambda→JSON，本地→可读
    let aws_config = aws_config::defaults(...).load().await;
    let config = ConfigLoader::load(&aws_config).await?;   // §2.1.5
    let seed = resolve_jwt_seed(&config)?;                 // SSM 或 dev 环境变量
    let jwt = Jwt::from_seed(&seed)?;
    let state = AppState { config, jwt, aws_config };
    let router = setup(state).await?;               // 冷启动构建一次，热请求复用
    let listener = TcpListener::bind(("0.0.0.0", PORT)).await?;
    axum::serve(listener, router).await?;
}
```

生命周期：**冷启动只执行一次**昂贵操作（配置加载、SSM 拉取、JWT 密钥、OIDC 发现），
热请求全部复用内存态。

#### 2.1.2 AppState / AppConfig `[已实现]`

```rust
pub struct AppState {
    pub config: AppConfig,          // 非敏感 + 已拉取的 secrets
    pub jwt: Jwt,                   // Ed25519 签名/验证器
    pub aws_config: aws_config::SdkConfig,  // 应用据此建 Dynamo/S3 客户端
}
pub struct AppConfig {
    pub project: String, pub environment: String, pub region: String,
    pub table_prefix: String,       // "operon-dev-" 环境隔离
    pub secrets: HashMap<String,String>,  // SSM 批量拉取结果
    pub dev_mode: bool,
}
```
通过 `FromRef` 让 `Jwt`/`AppConfig`/`SdkConfig` 可作为 axum State 提取器按需取用。

#### 2.1.3 中间件 `OLayer` `[已实现]`

```rust
// 对应文章 OLayer::new().request_id().tracing().cors().error_format()
impl OperonRouterExt for Router<S> {
    fn with_operon_defaults(self) -> Self {
        self.layer(TraceLayer::new_for_http())          // tracing
            .layer(SetRequestIdLayer::new(x-request-id, MakeRequestUuid))
            .layer(CorsLayer::permissive())             // 可配置
    }
}
```
统一错误由 `AppError` 的 `IntoResponse` 保证，不依赖中间件。

#### 2.1.4 统一错误 `AppError` `[已实现]`

```rust
pub enum AppError { Unauthorized/Forbidden/NotFound/BadRequest/Conflict/Internal(String) }
// IntoResponse → {"error":{"code","message"}} + 对应 HTTP 状态码
// impl From<anyhow::Error>、From<DynamoError>：业务代码直接 ?
```
`DynamoError → AppError` 的映射放在 dynamo crate（依赖 core），保持单向依赖。

#### 2.1.5 配置加载 `ConfigLoader` `[已实现]`

```
启动时:
  读环境变量（非敏感）：OPERON_PROJECT/ENV/TABLE_PREFIX/SECRETS_PATH
  若设置 OPERON_SECRETS_PATH:
    一次 GetParametersByPath(path, recursive, with_decryption)
    → secrets: HashMap<参数名末段, 值>    ← 冷启动一次，内存缓存
  dev 模式（无 SECRETS_PATH）：密钥从 OPERON_DEV_JWT_SEED 环境变量
```

**设计要点**：
- 一次 API 调用拉全部密钥，而非逐密钥请求。
- 密钥轮换 = 更新 SSM 参数，下次冷启动自动生效。
- `resolve_jwt_seed`: 优先 SSM `jwt_seed`(base64, 32B)，降级 dev 环境变量。

#### 2.1.6 认证

**JWT 自实现（Ed25519）`[已实现]`** —— 不依赖重型 JWT crate，密码学路径显式可控：

```
token = base64url(header) "." base64url(payload) "." base64url(signature)
header  = {"alg":"EdDSA","typ":"JWT"}
签名    = Ed25519(SHA-512 一次) over "h.p"  （RFC 8037）
验证    = 解析三段 → 验签 → 解析 payload → 检查 exp
```

```rust
pub struct Jwt { signing_key: SigningKey, verifying_key: VerifyingKey }
impl Jwt {
    pub fn from_seed(&[u8;32]) -> Self;
    pub fn sign(&self, &JwtClaims) -> Result<String>;
    pub fn verify(&self, &str) -> Result<JwtClaims, AppError>;  // 过期→Unauthorized
}
pub struct JwtClaims { sub, email, iat, exp, #[serde(flatten)] extra }
// 提取器 JwtAuth: 优先 X-Authorization，回退 Authorization（OAC 占用后者）
```

**API Key `[已实现]`**：`ApiKeyAuth` 提取器，`X-API-Key` 头 → SSM `api_key` → `subtle::ConstantTimeEq` 常量时间比较。

**OIDC `[已实现]`** —— 基于 **openidconnect**（通过 OpenID Relying Party Certification 官方认证）：

```
GET /api/auth/{provider}              → 302 到 IdP（PKCE + 加密 state cookie）
GET /api/auth/{provider}/callback     → 换 token → 库验证 ID Token → 业务 handler
GET /.well-known/jwks.json            → Ed25519 公钥端点
```

- Discovery / Authorization Code + PKCE / ID Token 验证（JWKS + exp/aud/iss/nonce）由 openidconnect 处理；
- `state`（csrf + nonce + pkce_verifier）用 **AES-256-GCM 加密进 cookie**，无需服务端存储；
- 成功后调用 `OidcAuthHandler`，业务签发自有 JWT。
框架 API（文章原文）：
```rust
let oidc = OidcRouter::builder()
    .base_url("https://myapp.example.com")
    .route_prefix("/api/auth")
    .cookie_key(key)
    .provider(google_config, MyAuthHandler)
    .build().await?;   // 内部做 .well-known 发现
let router = Router::new().merge(oidc.into_router()).with_state(state);
```

### 2.2 operon-dynamo `[已实现]`

```rust
pub struct DynamoClient { client: aws_sdk_dynamodb::Client, table: String }
impl DynamoClient {
    pub fn new(&SdkConfig, table: impl Into<String>) -> Self;
    pub async fn get<T: DeserializeOwned>(pk, sk) -> Result<Option<T>, DynamoError>;
    pub async fn put<T: Serialize>(&T) -> Result<(), DynamoError>;
    pub async fn delete(pk, sk) -> Result<(), DynamoError>;
    pub async fn query<T>(pk) -> Result<Vec<T>, DynamoError>;
}
// 序列化用 serde_dynamo（feature aws-sdk-dynamodb+1）
// 错误：NotFound / ConditionalCheckFailed / Aws；已映射到 AppError（404/409/500）
```
**薄封装边界**：无查询 DSL、无迁移工具。复杂操作直接走 SDK。
表名带环境前缀：`format!("{prefix}users")`，`prefix = "operon-dev-"`。

### 2.3 operon-s3 `[待实现]`

```rust
pub struct S3Client { client, bucket }
impl S3Client {
    pub async fn put_object(prefix, bytes, content_type) -> Result<String, S3Error>;
    pub async fn get_object(key) -> Result<Bytes, S3Error>;
    pub fn presign_get(key, expires) -> Result<Url, S3Error>;   // 预签名读
    pub fn presign_put(key, expires) -> Result<Url, S3Error>;   // 预签名写
}
```
路径约定层级化（`{project}/slides/{slide_id}/{image_id}.jpg`），前缀即"目录"，便于导出/删除。

### 2.4 operon-webhook `[待实现]`

```rust
#[async_trait]
pub trait WebhookVerifier: Send + Sync + 'static {
    type Event: DeserializeOwned + Send;
    async fn verify(&self, headers: &HeaderMap, body: &Bytes)
        -> Result<Self::Event, WebhookError>;
}
// 内置 StripeVerifier / GitHubVerifier / HmacVerifier；作为 axum 中间件在业务前验签
```

### 2.5 operon-cli `[部分实现]`

| 命令 | 实现状态 | 说明 |
|---|---|---|
| `operon gen-seed / dev-seed / token` | ✅ | 密钥生成、测试 JWT |
| `operon init --template <api\|fullstack\|webhook> <name>` | ✅ | 脚手架（`apps/` 下生成项目，路径依赖 operon crate） |
| `operon dev [--package]` | ✅ | 本地运行（`cargo run -p`，默认 operon-site） |
| `operon deploy --env <env> [--yes]` | ✅ | 调 `infra/deploy.sh`；prod 强制确认（`--yes` 跳过） |
| `operon logs --env <env>` | ✅ | CloudWatch 流式日志（`aws logs tail --follow`） |
| ARM64 优先交叉编译 | ⬜ | 默认 `aarch64-unknown-linux-musl`（待做） |

### 2.6 基础设施层 `[已实现：CFN 版]`

**设计目标**：从「600 行手写模板」到「80 行纯配置」。

`infra/template.yaml` 封装以下资源（等价文章 Pulumi npm 包的抽象面）：
- Lambda：runtime provided.al2023、arch、memory、timeout、Web Adapter layer、环境变量注入
- Function URL：AuthType AWS_IAM + CORS
- CloudFront：OAC（OriginType=lambda, sigv4, always）+ DefaultCacheBehavior
  - CachePolicy=CachingDisabled、OriginRequestPolicy=AllViewerExceptHostHeader
- DynamoDB 表、IAM Role、SSM 引用、日志组
- **两条 Lambda 权限**（InvokeFunctionUrl + InvokeFunction，source-arn 限 CloudFront）

`deploy.sh` 流程：SSM 密钥幂等创建 → musl 编译 → 打包 bootstrap.zip → 上传 → CFN deploy。

---

## 3. 关键设计决策（ADR）

| # | 决策 | 理由 | 状态 |
|---|---|---|---|
| ADR-1 | 单 Lambda 承载所有路由 | 冷启动品种唯一、部署单二进制、资源共享 | ✅ |
| ADR-2 | Function URL + CloudFront OAC 替代 API Gateway | 每百万请求省 $1；OAC 签名比裸端点安全 | ✅ |
| ADR-3 | 业务 JWT 走 X-Authorization | OAC 占用 Authorization 头（文章 4.2 原话） | ✅ |
| ADR-4 | 密钥入 SSM(SecureString)，冷启动一次批量拉取 | 密钥不落明文；一次调用缓存全部 | ✅ |
| ADR-5 | JWT 自实现（Ed25519）而非重型 crate | 密码学路径显式可控；RFC 7519/8037 简单 | ✅ |
| ADR-6 | musl 静态编译 | 本机 glibc 2.39 > AL2023 2.34，动态链接必崩 | ✅ |
| ADR-7 | IaC 用 CloudFormation（骨架）而非 Pulumi | 零额外工具链、幂等；与文章 Pulumi 方案等价 | ✅ |
| ADR-8 | API 不缓存（CachingDisabled） | 带认证接口若被缓存将返回陈旧数据 | ✅ |
| ADR-9 | Web Adapter 而非自研 Lambda Runtime | 保留完整 axum 能力、官方成熟方案 | ✅ |
| ADR-10 | OIDC state 用 AES-256-GCM 加密 cookie（openidconnect） | 无服务端状态存储 | ✅ |
| ADR-11 | DynamoDB 单表 pk/sk + GSI | 冗余换查询灵活性；表数最小化降低 IAM/监控面 | ✅ |

---

## 4. 数据模型（DynamoDB 单表 schema）

参考第五篇 slides 应用（当前骨架只用了 USERS 集合，此为通用扩展）：

| 实体 | PK | SK | GSI1-PK | GSI1-SK |
|---|---|---|---|---|
| 用户 | `USER#{id}` | `PROFILE` | — | — |
| 项目 | `PROJECT#{id}` | `META` | — | — |
| 项目成员 | `PROJECT#{id}` | `MEMBER#{uid}` | `USER#{uid}` | `PROJECT#{id}` |
| 幻灯片 | `PROJECT#{id}` | `SLIDE#{sid}` | — | — |
| 异步任务 | `PROJECT#{id}` | `JOB#{jid}` | `USER#{uid}` | `JOB#{created_at}`(按时间排序) |
| 使用记录 | `USER#{uid}` | `USAGE#{ts}` | — | — |

**约定**：`PK` 是实体的"从属根"，`SK` 表达关系；一条记录双写主表 + GSI1 获得两种查询视角；
TTL 字段自动清理（任务 7 天过期）。

---

## 5. 接口契约（关键签名）

| 接口 | 契约 |
|---|---|
| `run_with_setup` | 见 §2.1.1 |
| `JwtAuth` 提取器 | 读取 `X-Authorization: Bearer <jwt>`（回退 `Authorization`），验证失败 → 401 |
| `AppError` | 见 §2.1.4 |
| `DynamoClient` | 见 §2.2 |
| `WebhookVerifier` | 见 §2.4 |
| `OidcRouter::builder` | 见 §2.1.6 |
| `OperonRouterExt::with_operon_defaults` | 挂默认中间件 |

---

## 6. 安全设计

- **传输**：CloudFront HTTPS + OAC SigV4（源站不可公网直达）。
- **纵深防御**：边缘（CloudFront Function 验过期/提声明）→ 应用（完整验签）[第五篇，⬜]。
- **密钥**：SSM SecureString + KMS 解密；Lambda 环境变量无敏感信息。
- **认证**：JWT Ed25519 防篡改；过期检查；API Key 常量时间比较；OIDC 防 CSRF/nonce/时序。
- **错误**：统一 JSON，不泄露内部堆栈（AppError 只暴露 message）。
- **最小权限**：Lambda role 仅 Logs + 指定 DynamoDB 表 + 指定 SSM 路径 + KMS Decrypt。

---

## 7. 部署拓扑（当前骨架实际）

```
https://d3recyygcu2a3x.cloudfront.net         ← 对外
   │ CloudFront (OAC SigV4)
   ▼
Function URL (AWS_IAM)  jyyk44x….lambda-url.us-west-2.on.aws
   ▼
operon-dev-api Lambda (x86_64, 256MB, provided.al2023, Web Adapter)
   ├─ DynamoDB  operon-dev-users（pk/sk）
   ├─ SSM       /operon/dev/jwt_seed（SecureString）
   └─ S3        operon-deploy-…（部署包）
```

---

## 8. 性能与成本设计

| 目标 | 设计手段 | 实测 |
|---|---|---|
| 执行时间低 | Rust + 薄封装；热请求不重建资源 | 1~2ms（达标） |
| 冷启动可忍 | 单函数、内存缓存密钥、静态二进制 | init 301/347ms |
| 内存省 | Rust；36MB 占用 → 256MB 配置可降 128MB | 36MB |
| 成本线性 | DynamoDB 按需 + Lambda 按量 + CachingDisabled | ~$0.2/50万请求 |
| ARM64 -20% | 交叉编译（待做） | — |

---

## 9. 测试策略

- **单元**：core（JWT sign/verify、AppError 映射、ConfigLoader 解析）；dynamo（错误映射）。
- **集成**：本地起 axum 全路由冒烟（无 AWS 依赖，dev seed）。
- **E2E**（`docs/REQUIREMENTS.md §8` 验收清单）：部署后经 CloudFront 逐项验证。
- **性能**：CloudWatch REPORT 采样（Duration/Max Memory/INIT），阈值 ≥ 需求 §5。

---

## 10. 实现状态与路线图

```
✅ 已完成   core 运行时/中间件/JWT/API Key/OIDC/SQS | dynamo/webhook/s3 封装
           site 应用 + 模块化 | CLI 完整(init/dev/deploy/logs) | 29 单元测试
           CFN 基础设施 + deploy.sh | 自定义域名 + 证书续期 | 成本告警
⬜ 下一步   基础设施 npm 包(FR-9.4) → 边缘认证(FR-3.5) → ARM64 → 内存 256→128MB
```

每步实施建议遵循：**更新本 TDD → 让 Claude Code 实现 → musl 编译 + lint/test →
deploy staging → CloudWatch 验证 → 合入**（对应文章 9.1 的 SDD 闭环）。

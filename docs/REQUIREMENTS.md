# operon 框架需求文档

> **依据**：陈天《一人公司》系列文章（第三篇《打造低成本无服务器框架》为核心，
> 结合第五篇《当地基打好后如何快速盖楼》、收官篇《技术是基石赚钱才是硬道理》）。
> **目的**：把文章的观点提炼成可实施的需求规格，作为框架开发与验收的依据。
> **状态**：标注了当前骨架已实现 / 待实现（见 §7）。

---

## 1. 背景与目标

### 1.1 背景（文章观点）
一人公司（OPC）没有运维团队、没有值班轮换，后端服务如何跑？文章的答案是**无服务器（Serverless）**，
但市面通用框架塞满了用不上的抽象。因此作者自建了一个贴合自身基础设施的 Rust 无服务器框架
——**operon**。

### 1.2 目标
- **省心**：把运维心智负担降到零（不用打补丁、扩缩容、盯监控）。
- **省钱**：按需付费，让成本与流量线性挂钩，早期多实验服务账单不心疼。
- **AI 友好**：产出结构化日志，让 AI 编程助手能直接翻日志、读上下文、修 bug、重部署。
- **省时间**：用 AI + 规格驱动开发（SDD），让「自建框架」从团队级投入降到一天级。

### 1.3 成功标准（文章的量化锚点）
- 早期月请求 50 万次时，整套服务月成本 **≈ $2.5**（作者原文估算）。
- 从决策到上线（如 API Gateway → Function URL 迁移）**≈ 1 小时**。
- 新应用交付：**一个周末**从灵感到生产。

---

## 2. 核心设计理念（文章观点提炼）

| 观点 | 内涵 |
|---|---|
| **单 Lambda 打天下** | 所有 HTTP 路由放进一个 Lambda，axum Router 内部分发；冷启动「品种」唯一、部署只有一个二进制、资源（连接池/配置缓存）共享 |
| **Rust 优先** | 编译期质量保障、启动快、内存省、执行成本低（同等功能约为 Node 的 1/5~1/10）；Rust 类型系统是「AI 写的代码」的质量兜底 |
| **去除 API Gateway** | Function URL + CloudFront OAC 替代，每百万请求省 $1，且比裸 API Gateway 更安全 |
| **密钥永不明文** | 环境变量只放非敏感配置；敏感配置入 SSM（SecureString/KMS），冷启动一次批量拉取、内存缓存 |
| **框架即胶水** | 框架与特定基础设施深度耦合，才能发挥极致；「薄封装，不做 ORM」 |
| **让正确的用法成为最自然的用法** | API 面向人、更面向 AI 设计：builder 模式、pub(crate) 可见性管控、枚举校验、宏消除重复 |
| **基础设施投资呈 J 曲线** | 前期沉默，后期每个新应用边际成本趋近于零 |
| **为 AI 协作而设计** | 清晰的目录、集中的依赖、明确的 trait、声明式的部署配置，都是给 AI 的「施工图纸」 |

---

## 3. 总体架构需求

```
用户请求
  → CloudFront（边缘：OAC SigV4 签名 + 可选边缘认证）
    → Lambda Function URL（AWS_IAM，单函数）
      → Web Adapter → axum Router（单进程，多路由分发）
        → 中间件链（request_id → tracing → CORS → 统一错误）
        → 认证（JWT / API Key / OIDC）
        → 业务 handler
          → DynamoDB（薄封装）/ S3（对象+预签名）/ SQS（异步 worker）
```

- **一个 Lambda 承载所有 HTTP 路由**（FR-1）。
- **Function URL + CloudFront OAC** 对外暴露（FR-9）。
- 应用间共享能力通过 **Rust workspace 共享 crate**（golgi-core / golgi-ai / golgi-auth-edge 模式）。

---

## 4. 功能需求

### FR-1 运行时与启动
- FR-1.1 一行启动：`operon::run(router()).await` 完成日志初始化、配置加载、密钥解析、运行时启动。
- FR-1.2 支持异步路由构建：`run_with_setup(|state| async { ... })`，冷启动时执行一次（OIDC 发现、密钥初始化），热请求复用。
- FR-1.3 同一份二进制同时支持本地开发（监听 PORT）与 Lambda（Web Adapter / 自研 runtime）双模式。
- FR-1.4 提供 SQS 消费运行时 `run_sqs_with_setup`：消息反序列化、可见性超时、错误重试、幂等约定（第五篇）。

### FR-2 中间件（对应文章 `OLayer`）
- FR-2.1 请求追踪 ID（`request_id`）：每个请求生成并透传 `x-request-id`。
- FR-2.2 结构化日志（`tracing`）：Lambda 环境输出 JSON，本地输出可读格式。
- FR-2.3 CORS：`CorsConfig` 可配置，默认 `permissive()`。
- FR-2.4 统一错误格式（`error_format`）：所有错误 → `{"error":{"code","message"}}`，含正确 HTTP 状态码。
- FR-2.5 语义级配置校验：约束值用枚举（如 `Architecture::Arm64/X86_64`），解析期即报错。

### FR-3 认证体系（三层）
- FR-3.1 **JWT 无状态验证**：axum 提取器 `JwtAuth<Claims>`；支持 **EdDSA(Ed25519)、RS256、ES256**，默认推荐 EdDSA；JWKS 可托管于 S3 + CloudFront。
- FR-3.2 **API Key 验证**：服务间调用 `X-API-Key` 头，**常量时间比较**防时序攻击；密钥存 SSM。
- FR-3.3 **OIDC 登录**：Google/GitHub 等第三方 Authorization Code + **PKCE**；临时状态用 AES-256-GCM 加密进 cookie（无服务端存储）；CSRF/nonce/时序防护；完成后签发自有 JWT。
  - 框架自动生成：`/.well-known/jwks.json`、`/api/auth/{provider}`、`/api/auth/{provider}/callback`。
- FR-3.4 **用户 ID 映射**：JWT `sub` 用自有 UUID 而非第三方 ID，身份标识与身份提供商解耦（第五篇）。
- FR-3.5 **边缘认证（纵深防御，可选）**：CloudFront Function 先验过期时间/提取声明，Lambda 层再完整验证；图片等静态资源走 `/images/*` 边缘认证 + CDN 缓存。

### FR-4 配置管理（混合策略）
- FR-4.1 非敏感配置（项目名、环境、表前缀）走**环境变量**，IaC 部署时注入。
- FR-4.2 敏感配置（JWT 密钥、API Key、Webhook 密钥）存 **SSM 参数存储（SecureString/KMS）**。
- FR-4.3 冷启动**一次 `GetParametersByPath` 批量拉取**全部密钥并内存缓存，热请求零开销。
- FR-4.4 密钥轮换：只更新 SSM 参数，下次冷启动自动生效，无需重部署。

### FR-5 数据访问（DynamoDB 薄封装）
- FR-5.1 自动处理表名前缀（dev/prod 隔离）。
- FR-5.2 serde 自动序列化/反序列化。
- FR-5.3 统一错误映射：`ResourceNotFoundException` → HTTP 404 等。
- FR-5.4 批量操作自动重试 `UnprocessedKeys/UnprocessedItems`。
- FR-5.5 **不做 ORM**：无查询 DSL、无关系映射、无迁移工具；复杂操作直接用 SDK。
- FR-5.6 支持**单表设计**（pk/sk + GSI）：多实体共表、冗余换查询灵活性、TTL 自动清理（第五篇）。

### FR-6 对象存储（S3）
- FR-6.1 S3 操作封装：上传、读取、层级路径管理（`{project}/slides/{slide_id}/{image_id}.jpg`）。
- FR-6.2 **预签名 URL** 生成，用于直接上传/下载。
- FR-6.3 生命周期管理（临时导出文件自动清理）。

### FR-7 Webhook 验证（可插拔）
- FR-7.1 trait 化：`WebhookVerifier`，在请求体到达业务 handler 前完成签名验证（中间件）。
- FR-7.2 内置 **Stripe、GitHub、通用 HMAC** 三种验证器；支持自定义。
- FR-7.3 业务代码拿到的必是已验证数据，杜绝「忘验签名」。

### FR-8 CLI（`operon` 命令）
- FR-8.1 `operon init --template <api|fullstack|webhook> <name>`：项目脚手架。
- FR-8.2 `operon dev`：本地热重载开发。
- FR-8.3 `operon deploy --env <staging|prod>`：构建→打包→部署；**生产环境强制 preview + 人工确认**（`--yes` 跳过）。
- FR-8.4 `operon logs --env <env> --follow`：日志查看。
- FR-8.5 **ARM64 优先**：默认交叉编译 arm64（Graviton 便宜 ~20%）。
- FR-8.6 全局配置（AWS 账号、Pulumi 后端、证书 ARN、Route53 Zone）与项目配置（内存、表、前端构建）分离。

### FR-9 基础设施与部署（声明式）
- FR-9.1 项目用**声明式配置**（`operon.yaml` / `infra/index.ts`）描述全部基础设施：Lambda 内存/超时/架构、DynamoDB 表+GSI+TTL、SQS 队列（DLQ+重试）、S3 桶+CDN 行为+生命周期、CloudFront 多源多行为+边缘认证、ACM 证书、Route53、IAM、SSM、日志组。
- FR-9.2 **一次部署，整套基础设施从无到有**（2~3 分钟）。
- FR-9.3 IaC 采用**代码化**方案（文章用 Pulumi/TypeScript；本骨架用 CloudFormation 等价实现），支持封装/重构/分层。
- FR-9.4 基础设施逻辑抽成可复用 npm 包（`@scope/infra`），业务侧 `infra/index.ts` 只剩纯配置（文章「从 600 行模板到 80 行配置」）；基础设施升级与 CLI 发布解耦。
- FR-9.5 Function URL 配 **CloudFront + OAC**（SigV4 签名，仅经 CloudFront 的请求可达）；OAC 占用 `Authorization` 头，业务认证改用 `X-Authorization`。

### FR-10 应用层支持（workspace，第五篇）
- FR-10.1 共享能力 crate：通用类型（分页、用户身份、订阅等级）、AI provider trait、边缘认证生成。
- FR-10.2 每个应用 = API Lambda + 可选 SQS Worker（`lib.rs` 供 worker 复用，一份代码两个 Lambda）。
- FR-10.3 **先占坑后填坑**：数据模型/接口预留扩展点（订阅等级、角色、费用记录）。

---

## 5. 非功能需求

| 类别 | 需求 |
|---|---|
| **性能** | 热请求执行时间 ~1~2ms（骨架实测）；冷启动 init < 350ms 可接受；CloudFront 边缘认证响应 < 1ms |
| **成本** | 50 万请求/月总成本 ≤ $3（文章目标 $2.5）；成本与流量线性挂钩，无断崖跳变 |
| **内存** | 单请求内存占用应保持在 64MB 量级（骨架实测 36MB），256MB 配置可再降 |
| **安全** | 密钥不落明文配置；JWT 防篡改/过期；API Key 常量时间比较；OIDC 防 CSRF/nonce/时序攻击；纵深防御（边缘+应用双层） |
| **可维护性** | 薄封装、少抽象；模块 pub(crate)；依赖集中管理（workspace） |
| **AI 友好性** | 结构化 JSON 日志；清晰的目录/配置/trait 约定；`CLAUDE.md` 随项目演进 |

---

## 6. 架构约束（来自文章）

1. 无服务器优先：在需要长连接/WebSocket/超长计算任务之前，不引入容器或虚拟机。
2. 单函数模型：除非路由需求差异极大，否则不拆函数。
3. 成本优化的 80% 来自架构决策（Rust、ARM64、去 API Gateway），而非代码微调。
4. 框架只做「让 80% 场景舒服」，剩下 20% 交给 SDK 与 AI。
5. 一切为 AI 协作设计：规格文档 > 代码，接口明确 > 灵活。

---

## 7. 范围与优先级（MoSCoW + 实现状态）

> ✅ = 当前骨架已实现；⬜ = 待实现

### Must（必须）
- [✅] 单 Lambda + axum 多路由运行时（FR-1.1, 1.2, 1.3）
- [✅] 中间件：request_id / tracing / CORS / 统一错误（FR-2）
- [✅] JWT 无状态验证（Ed25519）（FR-3.1 部分）
- [✅] 混合配置：环境变量 + SSM 批量拉取（FR-4）
- [✅] DynamoDB 薄封装：表前缀、serde、错误映射、单表（FR-5 部分）
- [✅] Function URL + CloudFront OAC + 双权限（FR-9.5）
- [✅] 声明式基础设施模板 + `deploy.sh`（FR-9.1, 9.2 简化版）
- [✅] CLI 基础：gen-seed / token / dev-seed（FR-8 部分）

### Should（应该，框架的核心竞争力）
- [⬜] OIDC 登录（PKCE + 加密 cookie + JWKS 端点）（FR-3.3, 3.4）
- [⬜] API Key 常量时间验证（FR-3.2）
- [⬜] SQS Worker 运行时 `run_sqs_with_setup`（FR-1.4）
- [⬜] Webhook 验证器（Stripe/GitHub/HMAC）（FR-7）
- [⬜] S3 封装 + 预签名 URL（FR-6）
- [⬜] CLI 完整：init / dev / deploy / logs，ARM64 优先（FR-8）
- [⬜] 基础设施 npm 包抽象（FR-9.3, 9.4）
- [⬜] 边缘认证（CloudFront Function）（FR-3.5）

### Could（可以，按需）
- [⬜] RS256 / ES256 JWT 算法支持（FR-3.1）
- [⬜] 批量操作自动重试（FR-5.4）
- [⬜] 应用层 workspace 共享 crate（分页/用户身份/订阅等级）（FR-10）
- [⬜] operon 技能（SKILL.md）固化到 Claude Code（收官篇）

### Won't（本阶段不做）
- 容器 / K8s / 长连接 / WebSocket（违反约束 1）
- ORM、迁移工具（违反 FR-5.5）

---

## 8. 验收标准（可测试）

1. `cargo build --target x86_64-unknown-linux-musl` 零警告通过；本地 `cargo run` 各路由返回预期。
2. 单次 `./deploy.sh dev` 完成全套基础设施创建并输出端点。
3. 通过 CloudFront：
   - `GET /health` → 200；
   - 无认证访问受保护路由 → 401 统一 JSON；
   - 有效 JWT（`X-Authorization`）→ 200；篡改/过期/格式错 → 401；
   - `POST` 带 `x-amz-content-sha256` → 201，不带 → 403；
   - 裸访问 Function URL（无 SigV4）→ 403。
4. 性能：热请求 Lambda Duration ≤ 5ms（骨架实测 1~2ms）；内存 ≤ 64MB。
5. 成本：按 50 万请求/月场景核算 ≤ $3。
6. 密钥仅存在于 SSM，Lambda 配置/环境变量中无明文敏感信息。

---

## 9. 相关文档

- `README.md` —— 项目概览、使用、排坑
- `PROJECT-NOTES.md` —— 部署信息、性能实测
- `CLAUDE.md` —— AI 协作操作手册

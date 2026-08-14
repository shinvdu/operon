# CLAUDE.md

本项目是复现陈天《一人公司》系列文章技术方案的**端到端无服务器骨架**：
Rust + Lambda(Web Adapter) + axum，单 Lambda 打天下，Function URL + CloudFront OAC 架构。
完整文档见 `README.md`，部署与性能数据见 `PROJECT-NOTES.md`。

## 常用命令

```bash
# 构建（必须 musl 静态链接，兼容 Lambda AL2023；本机 glibc 过新）
cargo build --release --target x86_64-unknown-linux-musl -p operon-example

# 本地开发（无需 AWS）
export OPERON_DEV_JWT_SEED="$(cargo run -q -p operon-cli -- dev-seed)"
PORT=8080 cargo run -p operon-example

# CLI：生成密钥 / 签发测试 JWT
cargo run -q -p operon-cli -- gen-seed
cargo run -q -p operon-cli -- token --seed <base64seed> --sub user-123 --email test@example.com

# 端到端部署（编译→打包→上传→CloudFormation，含 CloudFront）
cd infra && ./deploy.sh dev
```

## 环境与凭证（重要）

- **AWS 账号** `317618187345`（IAM user `silas`），区域 **us-west-2**
- 凭证硬编码在 `infra/deploy.sh` 顶部（测试账号）
- **网络：上传/curl 必须走代理** `source ~/proxy.sh`（直连只有 ~50KB/s，走代理 ~3s/8MB）
- AWS CLI 大文件上传加 `--cli-read-timeout 290`，避免慢速超时
- Lambda 入口约定：可执行文件名为 `bootstrap`，zip 结构即 `bootstrap`

## 架构约定

- **单 Lambda + Web Adapter**：axum 监听 `PORT`(8080)，本地与 Lambda 同一份二进制
- **Function URL（AuthType=AWS_IAM）替代 API Gateway**；CloudFront + OAC 前置签名
- **密钥管理**：SSM SecureString（`/operon/dev/jwt_seed`），冷启动一次 `GetParametersByPath` 批量拉取缓存（`crates/core/src/config.rs`）
- **JWT（Ed25519）**：自实现 RFC 7519（`crates/core/src/auth.rs`）。**业务请求头用 `X-Authorization`**，因为 CloudFront OAC 会占用 `Authorization` 头
- **DynamoDB 薄封装**：`operon-dynamo`，单表 `pk`/`sk` 主键，自动表前缀 + serde + 统一错误映射
- **统一错误**：`AppError`（`crates/core/src/error.rs`）→ `{"error":{"code","message"}}`

## 代码约定

- **新增路由**：在 `apps/example/src/main.rs` 的 `Router::new()` 里 `.route(...)` 注册；handler 用 `State(state): State<AppState>` 取框架注入的状态，返回 `Result<Json<T>, AppError>`
- **新增数据模型**：`#[derive(Serialize, Deserialize, Clone)]`，字段含 `pk`/`sk`（单表键）
- **受保护路由**：handler 参数加 `JwtAuth(claims): JwtAuth`，框架自动从 `X-Authorization: Bearer <jwt>` 提取并验证
- **错误传播**：`dynamo` 的 `DynamoError` 已实现 `From -> AppError`，直接 `?`
- 新应用参照 `apps/example` 的结构（workspace 内）

## 部署排坑速查

1. **OAC 需要两条 Lambda 权限**（`lambda:InvokeFunctionUrl` + `lambda:InvokeFunction`），模板里已含，别删
2. **POST/PUT 客户端必须带 `x-amz-content-sha256`**（Lambda 不支持 unsigned payload），否则 403
3. **改 CloudFront 头转发**要确认 `OriginRequestPolicy`（自定义头如 X-Authorization 需 `AllViewerExceptHostHeader` = `b689b0a8-...`）
4. **缓存策略 ID**：`658327ea...` 是 CachingOptimized（缓存 24h）；API 用 **CachingDisabled** = `4135ea2d-6df8-44a3-9df3-4b5a84be39ad`
5. 改 Rust 代码后需完整走：`cargo build --target musl` → 打包 → 上传（走代理）→ `cloudformation deploy`
6. 完整排坑记录见 `README.md` 第八节

## 未做 / 后续

- OIDC 登录（PKCE + 加密 cookie）、Webhook 验证、SQS 异步 Worker 未实现
- ARM64 切换（`LambdaArchitecture=arm64` + 交叉编译，省 ~20%）
- Lambda 内存 256MB → 128MB（实测仅用 36MB）

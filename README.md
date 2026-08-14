# operon —— 一人公司无服务器框架 + Operon Cloud 公司网站

> 依据《一人公司》系列文章（陈小天）的技术方案复现。**当前线上跑着一个真实的公司网站**
> **Operon Cloud（磐云科技）**：一页宣传页 + 采购需求表单 + 管理员后台，
> 前后端分离（S3 静态 + Lambda API），全部走无服务器架构。

**线上地址**：`https://d3recyygcu2a3x.cloudfront.net`（宣传页）、`/admin.html`（管理员后台）
**用户流程**：访客填采购表单 → DynamoDB；管理员登录 → 查看全部需求。

## 一、这个骨架验证了什么

| 文章主张 | 骨架中的实现 | 状态 |
|---------|-------------|------|
| 单 Lambda 打天下（axum Router 分发所有路由） | `apps/example`，`/health` `/users` `/me` 全在一个函数 | ✅ |
| Function URL 替代 API Gateway（省 $1/百万请求） | `AWS::Lambda::Url`，`AuthType: AWS_IAM` | ✅ |
| CloudFront + OAC 前置，SigV4 签名才能到达 Lambda | `AWS::CloudFront::OriginAccessControl`（`OriginAccessControlOriginType: lambda`） | ✅ |
| SSM（SecureString/KMS）存密钥，冷启动批量拉取 | `ConfigLoader` 一次 `GetParametersByPath` 拉全部并缓存 | ✅ |
| JWT 无状态认证（EdDSA/Ed25519） | `crates/core/src/auth.rs` 自实现 RFC 7519 + RFC 8037 | ✅ |
| DynamoDB 薄封装（非 ORM，自动序列化/表前缀） | `crates/dynamo` | ✅ |
| 统一 JSON 错误格式 | `AppError` → `{"error":{"code","message"}}` | ✅ |
| CLI 脚手架 + 部署（`operon init/deploy`） | `operon-cli`（gen-seed / token / dev-seed）+ `infra/deploy.sh` | ✅（骨架版） |

## 二、与文章方案的差异（有意简化，便于测试）

1. **部署用 CloudFormation 而非 Pulumi(TypeScript)**：你机器上没有 Pulumi，CFN 幂等、零额外工具链，验证的是架构而非 IaC 工具。
2. **Lambda 用 Web Adapter 层承载 axum**（而非文章自研的 `operon::run()` 运行时）：Web Adapter 是 AWS 官方方案，把 HTTP 请求转发进 axum 监听的 8080 端口，行为等价、更成熟。
3. **架构为 x86_64**（本机可直接编译）；文章的 ARM64 便宜 ~20% 是后续一行参数的事（`LambdaArchitecture=arm64`，需交叉编译工具链）。
4. **未做 OIDC / Webhook / SQS worker**：OIDC 涉及完整 PKCE + 加密 cookie 状态机，属独立大模块，骨架聚焦核心架构验证。

## 三、项目结构

```
operon/
├── crates/
│   ├── core/        # 运行时引导、中间件、JWT、混合配置（环境变量 + SSM）
│   └── dynamo/      # DynamoDB 薄封装（自动表前缀、serde 序列化、错误映射）
├── apps/
│   ├── example/     # 骨架示例（/health /users /me，仅供学习）
│   └── site/        # 线上公司网站后端（/api/leads, /api/admin/*）
├── cli/             # operon-cli：gen-seed / token / dev-seed
├── frontend/        # 静态前端：index.html 宣传页 + admin.html 管理后台
└── infra/           # CloudFormation + deploy.sh（后端+前端一键部署）
```

## 四、本地开发（无 AWS 依赖跑通框架）

```bash
export OPERON_DEV_JWT_SEED="$(cargo run -q -p operon-cli -- dev-seed)"
PORT=8080 cargo run -p operon-example
# 另开终端：
curl localhost:8080/health
curl localhost:8080/me                          # → 401 统一 JSON 错误
TOKEN=$(cargo run -q -p operon-cli -- token --seed "$OPERON_DEV_JWT_SEED" --sub user-123 --email test@example.com)
curl -H "Authorization: Bearer $TOKEN" localhost:8080/me   # → claims
```

## 五、部署到 AWS（端到端）

```bash
cd infra
./deploy.sh dev     # 编译 → 打包 → 上传 → CloudFormation（含 CloudFront，几分钟）
```

### 测试
- **CloudFront 就绪后**（无需签名）：
  ```bash
  curl https://<dist>.cloudfront.net/health
  # POST 必须带 x-amz-content-sha256（Lambda 不支持 unsigned payload）
  BODY='{"email":"alice@example.com","name":"Alice"}'
  HASH=$(printf '%s' "$BODY" | sha256sum | cut -d' ' -f1)
  curl -X POST -H 'Content-Type: application/json' -H "x-amz-content-sha256: $HASH" \
       -d "$BODY" https://<dist>.cloudfront.net/users
  curl https://<dist>.cloudfront.net/users
  # JWT 走 X-Authorization（Authorization 被 CloudFront OAC 占用）
  SEED=$(aws ssm get-parameter --name /operon/dev/jwt_seed --with-decryption --query Parameter.Value --output text)
  TOKEN=$(cargo run -q -p operon-cli -- token --seed "$SEED" --sub user-123 --email test@example.com)
  curl -H "X-Authorization: Bearer $TOKEN" https://<dist>.cloudfront.net/me
  ```
- **Function URL 直连**（需 SigV4 签名，模拟公网裸访问被拒）：
  ```bash
  source ~/proxy.sh
  export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...
  python3 sigv4_request.py <FunctionUrl>/health
  ```

## 六、成本估算（对应文章第八节「一杯咖啡撑一个月」）

| 组件 | 月成本（测试期） |
|-----|----------------|
| Lambda（256MB，50ms，月几万次） | ~$0.02 |
| DynamoDB（按需，零流量零费用） | ~$0.01 |
| CloudFront（边缘，小额） | ~$0.10 |
| S3（代码包 + 日志） | ~$0.01 |
| SSM 参数（标准免费） | $0 |
| **合计** | **<$0.15/月** |

> 验证完成后建议清理：`aws cloudformation delete-stack --stack-name operon-dev`（见下）。

## 七、清理

```bash
aws cloudformation delete-stack --stack-name operon-dev
# 可选：aws s3 rb --force s3://operon-deploy-<account>-us-west-2
#       aws ssm delete-parameter --name /operon/dev/jwt_seed
```

## 八、实测发现与排坑记录

以下坑都是这次端到端验证中真实踩到并解决的，对复现文章方案很有价值：

1. **Rust 二进制要 musl 静态编译**。本机 glibc(2.39) 比 Lambda AL2023(2.34) 新，动态链接会 `GLIBC_2.38 not found`。
   `rustup target add x86_64-unknown-linux-musl` + `musl-tools`，用 `cargo build --target x86_64-unknown-linux-musl`。

2. **OAC 需要两条 Lambda 权限**：`lambda:InvokeFunctionUrl` **和** `lambda:InvokeFunction`。
   只加第一条，CloudFront 返回 `403 AccessDeniedException`。官方文档明确列出两条命令。

3. **CloudFront OAC 占用 `Authorization` header**（换成自己的 SigV4 签名）。这是文章 4.2 节的原话，实测复现：
   客户端发的 `Authorization: Bearer <jwt>` 到不了 Lambda，应用层收不到 → 401。
   **解法：业务 JWT 改走 `X-Authorization`**（框架代码已支持）。

4. **自定义 header 需要 `OriginRequestPolicy`**。默认 CachePolicy 不转发 `X-Authorization` 给 origin。
   加 `OriginRequestPolicyId: b689b0a8-53d0-40ab-baf2-68738e2966ac`（AllViewerExceptHostHeader）。

5. **托管 CachePolicy 的 ID 别记混**：`658327ea-f89d-4fab-a63d-7e88639e58f6` 是 **CachingOptimized**（缓存 24h，
   会缓存带认证接口的 200 响应，导致换 token 后仍返回旧 claims）；**CachingDisabled** 是
   `4135ea2d-6df8-44a3-9df3-4b5a84be39ad`。API 用 CachingDisabled。

6. **PUT/POST 必须带 `x-amz-content-sha256`**（Lambda 不支持 unsigned payload）。
   不带 → `403 InvalidSignatureException`。GET 不受影响。

7. **本环境网络**：走代理 `~/proxy.sh` 上传 S3 只要几秒；unset 代理直连反而 50KB/s 极慢。
   部署脚本顶部已 `source ~/proxy.sh`。

8. **冷启动**：Rust + Web Adapter + 静态二进制，冷启动 init 约 30-60ms（见 CloudWatch `INIT_REPORT`），
   印证文章「Rust 启动快」的论点。ARM64 可再省 ~20% 成本（改 `LambdaArchitecture=arm64` + 交叉编译）。


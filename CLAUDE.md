# CLAUDE.md

本项目基于陈天《一人公司》系列文章技术方案，当前线上跑的是 **Operon Cloud（磐云科技）公司网站**：
Rust + Lambda(Web Adapter) + axum 后端 + S3 静态前端，前后端分离，走 CloudFront。
完整文档见 `README.md`，部署与性能数据见 `PROJECT-NOTES.md`，需求/设计/API 见 `docs/`。

## 线上架构

```
CloudFront (arch.sky-city.me / d3recyygcu2a3x.cloudfront.net)
 ├─ 默认    → S3 静态网站（frontend/index.html, admin.html，OAC + bucket policy）
 └─ /api/* → Lambda operon-dev-site（单函数）
              ├─ POST /api/leads          采购需求 → DynamoDB operon-dev-leads（公开）
              ├─ POST /api/admin/login    管理员登录（SSM 密码）→ JWT
              └─ GET  /api/admin/leads    需求列表（JWT + admin role）
```

## 常用命令

```bash
# 构建（musl 静态链接，兼容 Lambda AL2023）
cargo build --release --target x86_64-unknown-linux-musl -p operon-site

# 本地开发（无需 AWS，密码用 OPERON_DEV_ADMIN_PASSWORD）
export OPERON_DEV_JWT_SEED="$(cargo run -q -p operon-cli -- dev-seed)"
export OPERON_DEV_ADMIN_PASSWORD="TestAdmin!123" PORT=8080
cargo run -p operon-site

# CLI 一条龙
cargo run -q -p operon-cli -- init --template api my-app   # 脚手架新应用
cargo run -q -p operon-cli -- dev                         # 本地运行
cargo run -q -p operon-cli -- deploy --env dev            # 部署（prod 需确认）
cargo run -q -p operon-cli -- logs --env dev              # CloudWatch 日志
cargo run -q -p operon-cli -- gen-seed                    # 生成密钥/密码
cargo run -q -p operon-cli -- token --seed <b64> --sub admin   # 签 JWT

# 端到端部署（编译→打包→上传后端+前端→CFN，含 CloudFront）
cd infra && ./deploy.sh dev
```

## 环境与凭证（重要）

- **AWS 账号** `317618187345`（IAM user `silas`），区域 **us-west-2**
- 凭证从项目根 `.env` 读取（`.env` 已 gitignore 排除，**不入库**；格式见 `.env.example`）
- **网络：上传/curl 必须走代理** `source ~/proxy.sh`（直连 ~50KB/s 极慢，走代理 ~3s/8MB）
- 管理员密码在 SSM `/operon/dev/admin_password`（首次部署自动生成并打印，明文不入库）

## 代码约定

### 后端 `apps/site/`（模块化，对应文章三层架构）

```
apps/site/src/
├── main.rs       # 入口：run_with_setup + Router 注册（唯一接触框架的地方）
├── models.rs     # 模型层：Lead / LeadRequest / LoginRequest 等 + DynamoDB 映射
└── handlers.rs   # 路由层：submit_lead / admin_login / admin_leads + 表名 helper
```

- **新增路由三步**：main.rs `Router::new()` 注册 → handlers.rs 加 handler → models.rs 加请求/响应结构
- **handler 约定**：`State(state): State<AppState>` 取框架注入状态，返回 `Result<Json<T>, AppError>`（统一错误 JSON）
- **数据模型**：`Lead` 单表（pk=`LEADS`, sk=零填充时间戳），表名自动带环境前缀（`operon-dev-leads`）
- **管理员**：内置 `admin`，密码存 SSM + `subtle::ConstantTimeEq` 常量时间比对，JWT 带 `role: admin`
- **认证**：受保护路由 handler 参数加 `JwtAuth(claims): JwtAuth`，自动从 `X-Authorization` 提取验证

### 前端 `frontend/`（原生 HTML/JS，无构建）

- index.html 宣传页+采购表单；admin.html 登录+需求列表
- POST 用 `crypto.subtle.digest('SHA-256')` 计算 `x-amz-content-sha256`（否则 403）
- 业务 JWT 走 `X-Authorization`（CloudFront OAC 占用 Authorization 头）

## 部署排坑速查

1. **OAC 需要两条 Lambda 权限**（`InvokeFunctionUrl` + `InvokeFunction`），模板已含，勿删
2. **S3 OAC 需要 bucket policy** 授信 CloudFront（模板已含 `FrontendBucketPolicy`）
3. **POST 必须带 `x-amz-content-sha256`**，否则 403
4. **缓存策略**：API/静态用 CachingDisabled=`4135ea2d-6df8-44a3-9df3-4b5a84be39ad`；自定义头转发需 OriginRequestPolicy=`b689b0a8-53d0-40ab-baf2-68738e2966ac`
5. 改代码完整流程：musl 编译 → 打包 → 上传（走代理）→ `cloudformation deploy` →（前端改了则重传 frontend/）
6. 完整排坑见 `README.md` 第八节

## 未做 / 后续

- OIDC 登录、Webhook、SQS Worker 未实现
- ARM64 切换（省 ~20%）、Lambda 内存 256→128MB（实测用 36MB）
- leads 状态流转（new → contacted → closed）未接管理界面按钮

# Operon Cloud 最终实施报告

> 报告日期：2026-08-14
> 项目定位：依据陈天《一人公司》系列文章技术方案，从零复现并落地为**真实可运营的公司网站**。
> 全流程：框架骨架 → 公司网站 → 自定义域名 → 全自动运维，均已上线验证。

---

## 一、项目概览

### 1.1 从文章到现实

陈天在《一人公司》系列中描述了一套技术主张：**单 Lambda 打天下、Rust + Serverless 省成本、AI 驱动开发、无流量不花钱**。
本项目将这些主张从纸面落地为线上真实运行的产品——**Operon Cloud（磐云科技）公司网站**。

### 1.2 最终成果

| 项 | 值 |
|---|---|
| 线上网站 | **`https://arch.sky-city.me`** |
| 管理后台 | `https://arch.sky-city.me/admin.html` |
| 后端 | Rust + Lambda（Web Adapter）单函数承载全部 API |
| 前端 | S3 静态托管（原生 HTML/JS，无构建链） |
| 证书 | Let's Encrypt（DNS-01 TXT，自动续期） |
| 无流量月成本 | **≈ $0.001**（约 1 分钱） |
| 成本告警 | $10/月 预算，邮件通知 163 邮箱 |

### 1.3 系列文章主张 → 落地对照

| 文章主张 | 落地情况 |
|---|---|
| 无服务器省钱省心 | ✅ Lambda + CloudFront + DynamoDB 全托管 |
| 单 Lambda 打天下 | ✅ `operon-dev-site` 一个函数承载全部路由 |
| Rust 执行成本低 | ✅ 实测热请求 1~2ms，内存 36MB |
| 无请求不花钱 | ✅ 无流量固定成本 ≈ $0.001/月 |
| AI 驱动快速交付 | ✅ 全流程 Claude Code 辅助，一个周末完成 |
| 密钥不明文 | ✅ SSM SecureString + KMS 加密 |

---

## 二、实施历程（四阶段）

### 阶段一：无服务器框架骨架
- 从零搭建 Rust workspace：`core`（运行时/中间件/JWT/配置）、`dynamo`（DynamoDB 薄封装）、`cli`
- 单 Lambda + axum，Function URL（AWS_IAM）替代 API Gateway，CloudFront + OAC 前置
- SSM 密钥冷启动批量拉取；musl 静态编译兼容 Lambda AL2023
- **验证**：全路由端到端跑通，性能达标（热请求 1~2ms、冷启动 ~300ms）

### 阶段二：Operon Cloud 公司网站
- 拟公司：磐云科技（无服务器云应用专家），一页宣传页 + 采购需求表单
- 后端 `apps/site`：采购需求提交、管理员登录（SSM 密码→JWT）、需求列表
- 前端 S3 静态托管（index.html + admin.html），CloudFront 双 origin（默认→S3，`/api/*`→Lambda）
- DynamoDB `operon-dev-leads` 单表存储采购需求
- **验证**：浏览器真实填表提交入库、管理员登录看到需求列表

### 阶段三：自定义域名 arch.sky-city.me
- 遭遇 NameSilo 两条硬限制（CNAME 拒下划线值、不支持 NS 记录）
- 改用 **Let's Encrypt + DNS-01 TXT + 导入 ACM** 绕开限制
- CloudFront 绑定 Aliases + ACM 证书（sni-only）
- **验证**：HTTPS 全链路访问，浏览器确认锁图标

### 阶段四：全自动运维
- **证书自动续期**：acme.sh cron → 60 天续期 → 自动导入 ACM → 更新 CloudFront → 清理旧证书
- **成本告警**：AWS Budgets（$10/月）+ SNS → 163 邮箱，超过 $8.5/$10 发邮件
- **验证**：续期闭环实际跑通、测试告警邮件成功送达

### 阶段五：能力补全与质量保障
- **CLI 完善**：`init` / `dev` / `deploy` / `logs` 一条龙（脚手架/本地运行/部署/日志）
- **后端模块化**：apps/site 拆为 main/models/handlers（对应文章三层架构）
- **单元测试**：29 个（JWT/错误映射/模型/认证/API Key/Webhook/S3/SQS/OIDC），约定新功能必须带测试
- **框架能力补全**：API Key 验证、Webhook 验证器、S3 封装（预签名 URL）、SQS Worker 运行时、OIDC（openidconnect）
- **认证落地**：OIDC（openidconnect，本地 E2E）+ **GitHub OAuth 2.0 登录**（`github.rs`，线上验证通过）
- **验证**：`cargo test` 全绿，核心链路回归通过

---

## 三、最终架构

```
访客 / 管理员
   │
   ▼
CloudFront  https://arch.sky-city.me
   ├── 默认    → S3 静态网站（index.html 宣传页 + admin.html 管理后台，OAC + bucket policy）
   └── /api/*  → Lambda operon-dev-site（Function URL, AWS_IAM）
                    ├─ GET  /health              健康检查
                    ├─ POST /api/leads           采购需求 → DynamoDB（公开）
                    ├─ POST /api/admin/login     管理员登录 → JWT（SSM 密码，常量时间比对）
                    ├─ GET  /api/my/leads        我的记录（JWT 登录用户）
                    └─ GET  /api/admin/leads     需求列表（JWT + role=admin）
                        │
                        ├─ DynamoDB  operon-dev-leads（pk=LEADS, sk=时间戳）
                        ├─ SSM      jwt_seed / admin_password / acm_cert_arn
                        └─ 证书      ACM us-east-1（Let's Encrypt 导入）

自动运维：
  acme.sh cron ──60天续期──► renew-acm.sh（导入ACM→更新CloudFront→同步CFN→清理旧证书）
  AWS Budgets $10/月 ──超阈值──► SNS ──► 邮件 18217401108@163.com
```

### 资源清单

| 资源 | 名称 | 说明 |
|---|---|---|
| Lambda | `operon-dev-site` | x86_64, 256MB, provided.al2023 + Web Adapter |
| DynamoDB | `operon-dev-leads` | 按需计费，单表 pk/sk + PITR |
| S3 | `operon-dev-frontend` | 静态前端（OAC 私有访问） |
| S3 | `operon-deploy-*` | 部署代码包 |
| CloudFront | `E1NYOKR46XYB9U` | 双 origin + OAC + ACM 证书 |
| SSM | `/operon/dev/{jwt_seed,admin_password,acm_cert_arn}` | SecureString |
| ACM | 导入证书 | us-east-1，CloudFront 专用 |
| Budgets | `operon-monthly-budget` | $10/月 |

---

## 四、功能与验证结果

### 4.1 网站功能（浏览器实测通过）
- ✅ 一页宣传页（公司介绍/服务/优势/表单）渲染正常
- ✅ 普通用户登录（Google/GitHub）+ 我的记录（登录后查看自己提交的需求）
- ✅ 采购需求表单真实提交 → 入库 DynamoDB
- ✅ 管理员登录（JWT）→ 查看需求列表
- ✅ 自定义域名 HTTPS 访问（证书有效）

### 4.2 API 验证矩阵

| 场景 | 结果 |
|---|---|
| `GET /health` | 200 ✅ |
| `POST /api/leads`（带 body hash） | 201 入库 ✅ |
| `POST /api/leads`（缺 hash） | 403（Function URL 限制）✅ |
| `GET /api/admin/leads`（无 token） | 401 ✅ |
| `GET /api/admin/leads`（有效 JWT） | 200 列表 ✅ |
| 篡改/过期/格式错 token | 401 ✅ |
| 裸访问 Function URL（无 SigV4） | 403（AWS_IAM 生效）✅ |

### 4.3 性能实测

| 指标 | 实测 |
|---|---|
| 热请求执行（Lambda） | 1~2ms |
| 冷启动 init | ~300ms |
| 内存占用 | 35~36MB（配置 256MB） |
| 端到端（本机走代理） | P50 ~920ms（网络占 99%，应用 1~2ms） |

---

## 五、成本分析

| 场景 | 月成本 | 说明 |
|---|---|---|
| **无流量** | **≈ $0.001** | 仅 S3 32MB 部署包存储 |
| 50 万请求/月 | ≈ $0.20 | Lambda 2ms + DynamoDB + CloudFront |
| 成本告警配置 | $0 | 1 个预算免费 + SNS 免费层级 |

> 详细核算见 `docs/COST.md`。已清理历史 ECR 仓库（$0.0025/月残留），账单彻底干净。

---

## 六、安全设计

- **传输安全**：CloudFront HTTPS + 双 OAC（S3 + Lambda）SigV4 签名，源站不可公网直达
- **密钥管理**：SSM SecureString + KMS 加密，环境变量无明文敏感信息
- **认证**：JWT Ed25519（自实现 RFC 7519）+ 常量时间密码比对 + role 校验
- **业务 JWT 走 `X-Authorization`**（OAC 占用 Authorization 头）
- **最小权限**：Lambda role 仅 Logs + 指定 DynamoDB 表 + 指定 SSM 路径

---

## 七、运维自动化

| 自动化 | 机制 | 状态 |
|---|---|---|
| 证书续期 | acme.sh cron（每天 4 次检查，60 天续期）→ `renew-acm.sh` 自动导入/切换/清理 | ✅ 闭环验证 |
| 成本告警 | AWS Budgets $10/月 → SNS → 163 邮箱 | ✅ 测试邮件送达 |
| 一键部署 | `infra/deploy.sh`（编译→打包→上传前后端→CFN） | ✅ |
| 质量保障 | `cargo test` 26 个测试全绿（CLAUDE.md 约定新功能必带测试） | ✅ |
| 文档 | README / CLAUDE / PROJECT-NOTES / REQUIREMENTS / DESIGN / API / COST / AWS-OPERATIONS / FINAL-REPORT | ✅ |

---

## 八、关键排坑经验（防再踩）

1. **Rust 二进制要 musl 静态编译**（本机 glibc > Lambda AL2023）
2. **OAC 需两条 Lambda 权限**（InvokeFunctionUrl + InvokeFunction）
3. **S3 OAC 需要 bucket policy**（且手动加 + CFN 管理会冲突，统一 CFN）
4. **CloudFront OAC 占用 Authorization 头** → 业务 JWT 走 X-Authorization
5. **POST 必须带 x-amz-content-sha256**（Function URL 不支持 unsigned payload）
6. **CachePolicy ID 别记混**：CachingDisabled=`4135ea2d-...`，CachingOptimized=`658327ea-...`
7. **自定义头转发需 OriginRequestPolicy**（AllViewerExceptHostHeader）
8. **NameSilo 限制**：CNAME 拒下划线值、不支持 NS 记录 → Let's Encrypt TXT 绕开
9. **`aws acm import-certificate --certificate file://` 有 bug** → 用 boto3 直接传 bytes
10. **`list-certificates` 对 imported 证书不显示** → 清理用本地历史文件
11. **网络**：上传/curl 必须走代理 `~/proxy.sh`（直连 50KB/s 极慢）

---

## 九、Git 版本记录

```
7a4c950 feat(core): 补齐 API Key / Webhook / S3 / SQS Worker 框架能力
42119ab test(core): 添加核心单元测试
2e8cdf0 refactor(site): 后端模块化拆分
9a2196a feat(cli): 实现 init/dev/deploy/logs 命令
9c70099 chore(infra): 凭证移入 .env 环境变量
bce948d feat(infra): 证书自动续期脚本（Let's Encrypt → ACM → CloudFront 全自动）
ca21744 feat(site): 绑定自定义域名 arch.sky-city.me（Let's Encrypt + ACM）
47eb2f0 feat(site): Operon Cloud 公司网站上线（前后端分离）
b6872fd feat(operon): 搭建一人公司无服务器框架端到端骨架
```

---

## 十、后续建议

- [x] OIDC 登录（openidconnect，本地 E2E）+ GitHub OAuth 2.0（线上验证，2026-08 完成）
- [ ] 基础设施 npm 包抽象（CFN 模板 → 可复用 npm 包，文章「600 行到 80 行」）
- [ ] 边缘认证（CloudFront Function 纵深防御，第一层边缘验 JWT）
- [ ] Lambda 内存 256MB → 128MB（实测仅用 36MB，成本再降一半）
- [ ] ARM64 切换（交叉编译，Lambda 省 ~20%）
- [ ] leads 状态流转（new→contacted→closed）管理界面
- [ ] staging/prod 环境隔离部署

---

## 附：相关文档索引

| 文档 | 内容 |
|---|---|
| `README.md` | 项目概览、使用、排坑 |
| `PROJECT-NOTES.md` | 部署信息、性能、运维记录 |
| `CLAUDE.md` | AI 协作操作手册 |
| `docs/REQUIREMENTS.md` | 需求规格（MoSCoW） |
| `docs/DESIGN.md` | 技术设计（ADR） |
| `docs/API.md` | 接口文档（curl 示例） |
| `docs/COST.md` | 成本核算 |
| `docs/OIDC.md` | OIDC/GitHub 登录实现与配置指南 |
| `docs/AWS-OPERATIONS.md` | AWS CLI 操作手册（证书/创建/更新/测试/告警/清理） |
| `docs/FINAL-REPORT.md` | 本报告 |

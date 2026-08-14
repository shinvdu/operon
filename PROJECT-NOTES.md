# operon 项目记录（PROJECT NOTES）

> 依据《一人公司》系列文章复现的端到端无服务器骨架。本文件记录部署信息、
> 架构决策、性能实测数据与待办事项。技术细节与排坑见 `README.md`。

## 一、部署信息（2026-08-14 实测，当前为 Operon Cloud 公司网站）

| 项 | 值 |
|---|---|
| AWS 账号 | 317618187345（IAM user `silas`，AdministratorAccess） |
| 区域 | us-west-2（CloudFront 证书需 us-east-1） |
| 网站 | **`https://arch.sky-city.me`**（自定义域名） |
| 管理后台 | `https://arch.sky-city.me/admin.html` |
| 回退域名 | `https://d3recyygcu2a3x.cloudfront.net`（分布 ID `E1NYOKR46XYB9U`） |
| Lambda Function URL | `https://3a4fsjkdf3v4kjgd6ogw65be7y0bbuxi.lambda-url.us-west-2.on.aws/` |
| Lambda 函数 | `operon-dev-site`（x86_64, 256MB, provided.al2023 + Web Adapter） |
| DynamoDB | `operon-dev-leads`（采购需求，pk=`LEADS`/sk=时间戳，按需计费） |
| S3 前端 | `operon-dev-frontend`（index.html + admin.html，OAC + bucket policy） |
| S3 部署桶 | `operon-deploy-317618187345-us-west-2` |
| SSM 密钥 | `/operon/dev/jwt_seed`、`/operon/dev/admin_password`（均 SecureString） |

## 二、关键架构决策（已验证）

1. **单 Lambda 打天下**：所有路由在一个 axum 应用（Web Adapter 层承载，Lambda 监听 8080）
2. **Function URL（AWS_IAM）替代 API Gateway**：公网裸访问被拒，SigV4 签名放行
3. **CloudFront + OAC** 前置：SigV4 签名链路，OAC OriginType=`lambda`，SigningBehavior=`always`
4. **SSM SecureString 存密钥**：冷启动一次 `GetParametersByPath` 批量拉取缓存
5. **JWT 认证（Ed25519）**：自实现 RFC 7519，业务走 `X-Authorization` 头
6. **DynamoDB 薄封装**：非 ORM，自动表前缀 + serde 序列化 + 统一错误映射
7. **musl 静态编译**：`x86_64-unknown-linux-musl`，兼容 Lambda AL2023（本机 glibc 2.39 过新）

## 三、性能实测数据（2026-08-14）

### Lambda 应用层（CloudWatch REPORT）
| 指标 | 实测 | 备注 |
|---|---|---|
| 热请求执行 | **1~2ms**（/health、/me） | 文章成本表按平均 50ms 估算，远优于假设 |
| 带 DynamoDB 请求 | 20~60ms（/users） | 含 DynamoDB 网络 RTT |
| 冷启动 init | **301ms / 347ms** | 含 Web Adapter 层 + SSM 密钥拉取 |
| 内存占用 | **35~36MB**（配置 256MB） | 可降到 128MB |
| 计费时长 | 大量请求仅 **2ms** | 1ms 粒度计费 |

### 端到端延迟（含网络，非应用）
- P50 ≈ 920ms，P95 ≈ 3.1s
- 延迟分解：TLS 握手 ~800ms（代理 → 法兰克福边缘 → 美西），**网络占 99%**
- Lambda 本身仅 1~2ms；真实用户若在美西端到端约 ~100ms 量级

### 成本验证（按文章 50 万请求/月场景）
```
Lambda = 50万 × 2ms × 0.25GB = 250 GB-s → ~$0.004 + 50万调用费 ~$0.10
DynamoDB / CloudFront / S3 / SSM        ≈ ~$0.12
合计 ≈ $0.2/月（文章按 50ms 假设估 $2.5/月，实测更低）
```

## 四、排坑要点（详见 README 第八节）

1. OAC 需**两条** Lambda 权限：`lambda:InvokeFunctionUrl` + `lambda:InvokeFunction`
2. OAC 占用 `Authorization` 头 → 业务 JWT 用 `X-Authorization`（复现文章 4.2 节）
3. 自定义头转发需 `OriginRequestPolicy`（`AllViewerExceptHostHeader` = `b689b0a8-53d0-40ab-baf2-68738e2966ac`）
4. CachePolicy ID 坑：`658327ea...` 是 CachingOptimized（缓存 24h）；CachingDisabled 是 `4135ea2d-6df8-44a3-9df3-4b5a84be39ad`
5. PUT/POST 必须带 `x-amz-content-sha256`（Lambda 不支持 unsigned payload）
6. **上传走代理 `~/proxy.sh` 只需 ~3s**；unset 代理直连仅 50KB/s（部署脚本已处理）

## 五、待办 / 后续优化

- [ ] 内存 256MB → 128MB（实测仅用 36MB，成本再降一半）
- [ ] ARM64 切换（`LambdaArchitecture=arm64` + 交叉编译，省 ~20%）
- [ ] OIDC 登录（PKCE + 加密 cookie）——独立大模块
- [ ] Webhook 验证（Stripe/GitHub/HMAC）——可插拔 trait 已预留思路
- [ ] SQS 异步 Worker（文章第五篇 slides-worker 模式）
- [ ] 性能复测建议在美西区域就近执行，排除本机网络因素
- [ ] leads 状态流转（new→contacted→closed）管理按钮
- [x] 自定义域名 arch.sky-city.me + 证书自动续期（2026-08-14 完成）
- [ ] staging/prod 环境隔离部署

## 六、Operon Cloud 公司网站（2026-08-14 上线）

**业务**：磐云科技（Operon Cloud）——无服务器云应用专家（拟名，贴合 operon 定位）

**用户流程**（全部浏览器实测通过）：
1. 访客打开首页 → 填写采购需求表单 → `POST /api/leads` → DynamoDB
2. 管理员打开 `/admin.html` → 登录（SSM 密码，常量时间比对）→ JWT → 查看需求列表

**API 路由**（单 Lambda `apps/site`）：
| 方法/路径 | 说明 | 认证 |
|---|---|---|
| `GET /health` | 健康检查 | 无 |
| `POST /api/leads` | 提交采购需求（需 `x-amz-content-sha256`） | 无 |
| `POST /api/admin/login` | 管理员登录（SSM 密码→JWT，8h 有效） | 无 |
| `GET /api/admin/leads` | 需求列表（时间倒序） | JWT + `role=admin` |

**管理员密码**：存于 SSM `/operon/dev/admin_password`（部署时自动生成，明文不入库）。
查看：`aws ssm get-parameter --name /operon/dev/admin_password --with-decryption --query Parameter.Value --output text`
修改：`aws ssm put-parameter --name /operon/dev/admin_password --type SecureString --value <新密码> --overwrite`

**实测结果**：浏览器真实填表提交"云上科技有限公司"需求已入库；管理员浏览器登录后看到 2 条需求（含时间/联系方式/需求/预算/状态）。

**本次新增排坑**：
- **S3 OAC 需要 bucket policy** 授信 CloudFront（`FrontendBucketPolicy`，否则 403 AccessDenied）
- 前端 POST 用 `crypto.subtle.digest('SHA-256')` 计算 `x-amz-content-sha256`（Function URL 必需）

## 六之补充、自定义域名 arch.sky-city.me（2026-08-14 上线）

**最终方案**：Let's Encrypt（DNS-01 TXT 挑战）→ 导入 ACM → CloudFront 绑定。
绕开了 NameSilo 的两条硬限制。

**NameSilo 限制（重要，防止再踩）**：
- CNAME 的 **rrvalue 拒绝以下划线开头** → ACM 的 `_xxx.acm-validations.aws` 验证值全部被拒（210）
- **不支持 NS 记录**（只能 A/AAAA/CNAME/MX/TXT/SRV/CAA）→ Route53 子域委托失败
- 支持 TXT（含下划线 host）→ 这是最终方案的突破口

**实施步骤**：
1. `acme.sh --issue --dns dns_namesilo -d arch.sky-city.me`（内置 NameSilo hook，TXT 自动加/删）
2. 证书：`~/.acme.sh/arch.sky-city.me_ecc/`（自动续期 2026-10-13，cron 已装）
3. **导入 ACM**：`boto3 acm.import_certificate(Certificate=leaf, PrivateKey=key, CertificateChain=ca.cer)`，region **us-east-1**
   - ⚠️ **不要用 `aws acm import-certificate --certificate file://`**——AWS CLI 对 blob 参数有解析 bug（报"多证书"/Invalid base64）；用 boto3 直接传 bytes
4. CloudFront：`Aliases=[arch.sky-city.me]` + `ViewerCertificate: AcmCertificateArn`（sni-only, TLSv1.2_2021）
5. NameSilo 最终 CNAME：`arch → d3recyygcu2a3x.cloudfront.net`

**已放弃的弯路**：ACM DNS 验证（NameSilo 拒下划线 CNAME）、Email 验证（arch 子域无 MX）、Route53 委托（NameSilo 无 NS）、IAM 证书（CloudFront 报证书无效；且 IamCertificateId 只收 ≤32 字符 ID）。

## 六之补充2、证书自动续期（2026-08-14 配置完成，闭环已验证）

**架构**：acme.sh cron（每天 4 次检查）→ 60 天自动续期 → `reloadcmd` 自动执行 `infra/renew-acm.sh`

**renew-acm.sh 流程**（已实际跑通验证）：
1. 导入 ACM（us-east-1，boto3 直接传 bytes）→ 新 ARN
2. 更新 CloudFront `ViewerCertificate`（boto3 `update_distribution`，秒级生效）
3. 同步 CFN 栈参数 `AcmCertificateArn`（避免下次 `cloudformation deploy` 回滚证书）
4. 清理旧证书：历史文件 `~/.acme.sh/<domain>_renew_history` 跟踪 ARN，只留最新

**当前证书**：`arn:aws:acm:us-east-1:317618187345:certificate/e1ec7bd5-...`（2026-11 到期，到期前自动续期切换）

**组件位置**：
| 组件 | 位置/配置 |
|---|---|
| acme.sh cron | `57 1,7,13,19 * * * ~/.acme.sh/acme.sh --cron`（每天 4 次） |
| reloadcmd | `~/.acme.sh/arch.sky-city.me_ecc/arch.sky-city.me.conf` 的 `Le_ReloadCmd`（base64 存脚本路径） |
| 续期脚本 | `infra/renew-acm.sh`（需 boto3 + 走代理 + 凭证） |
| SSM 记录 | `/operon/dev/acm_cert_arn`（当前在用证书 ARN） |

**新增排坑**：
- **`list-certificates` 对 imported 证书返回空**（ACM 怪癖）→ 清理改用历史文件方案，别用 list 找证书
- S3 bucket policy：**手动 put + CFN 模板同时管理会冲突**（"policy already exists"）→ 统一由 CFN 管理
- 脚本里 aws CLI 命令要 `export AWS_DEFAULT_REGION`（boto3 用显式 region，CLI 用默认，易漏）

**手动触发续期**：`acme.sh --renew -d arch.sky-city.me --force`（会自动走 renew-acm.sh）

## 七、相关命令

```bash
# 部署（编译 site + 上传后端 + 上传前端 + CFN）
cd infra && ./deploy.sh dev
# 签发测试 JWT
cargo run -q -p operon-cli -- token --seed "$(aws ssm get-parameter --name /operon/dev/jwt_seed --with-decryption --query Parameter.Value --output text)" --sub user-123 --email test@example.com
# 查看网站 URL
aws cloudformation describe-stacks --stack-name operon-dev --query "Stacks[0].Outputs[?OutputKey=='CloudFrontUrl'].OutputValue" --output text
# 清理
aws cloudformation delete-stack --stack-name operon-dev
```

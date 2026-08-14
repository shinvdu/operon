# operon 项目记录（PROJECT NOTES）

> 依据《一人公司》系列文章复现的端到端无服务器骨架。本文件记录部署信息、
> 架构决策、性能实测数据与待办事项。技术细节与排坑见 `README.md`。

## 一、部署信息（2026-08-14 实测）

| 项 | 值 |
|---|---|
| AWS 账号 | 317618187345（IAM user `silas`，AdministratorAccess） |
| 区域 | us-west-2 |
| CloudFront | `https://d3recyygcu2a3x.cloudfront.net`（分布 ID `E1NYOKR46XYB9U`） |
| Lambda Function URL | `https://jyyk44xtwr5oriprkrpnlkb56i0bcsny.lambda-url.us-west-2.on.aws/` |
| Lambda 函数 | `operon-dev-api`（x86_64, 256MB, provided.al2023 + Web Adapter 层） |
| DynamoDB | `operon-dev-users`（单表 pk/sk，按需计费） |
| SSM 密钥 | `/operon/dev/jwt_seed`（SecureString，Ed25519 seed） |
| S3 部署桶 | `operon-deploy-317618187345-us-west-2` |

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

## 六、相关命令

```bash
# 部署
cd infra && ./deploy.sh dev
# 签发测试 JWT
cargo run -q -p operon-cli -- token --seed "$(aws ssm get-parameter --name /operon/dev/jwt_seed --with-decryption --query Parameter.Value --output text)" --sub user-123 --email test@example.com
# 清理
aws cloudformation delete-stack --stack-name operon-dev
```

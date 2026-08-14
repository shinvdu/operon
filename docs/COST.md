# operon 成本核算（COST）

> 当前部署（Operon Cloud 公司网站）的 AWS 费用分析。核算日期：2026-08-14。
> 定价区域：us-west-2（Lambda/DynamoDB/S3/SSM/Logs），us-east-1（ACM，CloudFront 证书）。

---

## 一、无流量时的固定成本（约 **$0.001/月**，接近 0 元）

无流量 = 零请求、零访问。此时**只有存储类资源**收费，全部按量计费的服务（Lambda/DynamoDB/CloudFront）在无请求时成本为 0。

| 资源 | 计费模型 | 无流量时费用 |
|---|---|---|
| Lambda `operon-dev-site`（256MB, x86_64） | 按调用+执行时长，无保留并发 | **$0**（函数存在不收费） |
| CloudFront distribution（arch.sky-city.me） | 按请求+传输，无月费 | **$0** |
| DynamoDB `operon-dev-leads`（按需） | 按读写请求 + 存储 + PITR | **≈ $0**（无请求；约 3 条记录 ≈1KB → 存储费 ≈ $0.0000003） |
| S3 `operon-dev-frontend` + `operon-deploy-*` | 存储按 GB | **≈ $0.001**（约 32MB 部署包 + 20KB 前端 ≈ 0.03GB × $0.023） |
| SSM 参数（jwt_seed / admin_password / acm_cert_arn） | 标准参数 | **$0**（每账户前 1 万个免费） |
| CloudWatch Logs | 按写入 + 存储 | **≈ $0**（无流量无写入；已有日志几 KB） |
| ACM 导入证书（CloudFront 用） | — | **$0** |
| IAM / Route53 | — | **$0**（无 hosted zone） |
| **合计** | | **≈ $0.001/月** |

> 结论：无流量时**唯一持续成本**是 S3 里那 32MB 部署包（约 $0.001/月），其余全 $0。
> 这完全验证了文章「没请求就不花钱」的主张。

---

## 二、有流量时的按量成本（估算）

按文章场景：**月请求 50 万次**，256MB 平均执行（实测 1~2ms）：

| 服务 | 计算 | 月费用 |
|---|---|---|
| Lambda | 50万 × 2ms × 0.25GB = 250 GB-s → $0.0000167/GB-s | ~$0.004 |
| Lambda 请求费 | 50万 × $0.20/百万 | ~$0.10 |
| DynamoDB 按需（写 1 万 + 读 10 万） | 按量，极少量级 | ~$0.01 |
| CloudFront（50万请求 + 1GB 传输） | $0.0075/万请求 + $0.085/GB | ~$0.05 |
| S3 / SSM / Logs | 同上 | ~$0.01 |
| **合计** | | **≈ $0.20/月** |

> 比文章原估 $2.5/月 低一个数量级，原因：
> - 实测执行 1~2ms（文章按 50ms 估）→ Lambda 计费时长低 25 倍
> - 内存 256MB（实际占用 36MB，还可降到 128MB）
> - DynamoDB 数据量极小

---

## 三、当前实际账单（Cost Explorer，2026-08-01 ~ 08-14）

```
Amazon EC2 Container Registry (ECR)   $0.0025   ← 历史残留，与当前服务无关
Lambda / DynamoDB / S3 / CloudFront / SSM / Logs  < $0.0001（未显示 = 几乎为 0）
```

当月累计：**≈ $0.0025**（全部来自历史 ECR 残留，非当前部署）。

---

## 四、与文章成本主张对比

| 文章 | 实测 |
|---|---|
| 月请求 50 万次总成本 ≈ $2.5 | ≈ $0.20（执行时间低 25 倍） |
| 无请求不花钱 | ✅ 无流量 ≈ $0.001 |
| 成本与流量线性挂钩 | ✅ Lambda 按量 / DynamoDB 按需 |
| 选择 Rust 成本约为 Node 1/5~1/10 | ✅ 1~2ms 执行 + 36MB 内存 |

---

## 五、进一步优化（可选）

1. **Lambda 内存 256MB → 128MB**：实测仅用 36MB，计费时长成本再降一半
2. **清理 S3 历史部署包**：`operon-deploy-` 桶只留最新 bootstrap.zip，省那 $0.001
3. **ARM64（Graviton）**：交叉编译，Lambda 成本省 ~20%
4. **DynamoDB 关闭 PITR**（若不重要）：省表大小 $0.20/GB 的增量（当前数据量小，影响可忽略）

---

## 六、清理所有资源（彻底停止计费）

```bash
aws cloudformation delete-stack --stack-name operon-dev      # 删 Lambda/DynamoDB/S3/CloudFront
aws s3 rb --force s3://operon-deploy-317618187345-us-west-2  # 删部署桶
# 域名 DNS（NameSilo）与 Let's Encrypt 证书可保留，成本 0
```

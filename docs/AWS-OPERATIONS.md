# Operon Cloud AWS CLI 操作手册

> 本项目从创建到运维用到的全部 AWS CLI / 相关命令，按「证书 → 创建 → 更新 → 测试 → 告警 → 清理」组织。
> 均经实际执行验证。适用环境：本机（Linux），测试账号。
> 项目源码与一键脚本见 `../infra/`（deploy.sh / renew-acm.sh / sigv4_request.py / presign_put.py）。

---

## 0. 环境准备

```bash
# 代理（关键：上传/访问走代理，直连 50KB/s 极慢）
source ~/proxy.sh

# 凭证（测试账号，可 export 或写进脚本）
export AWS_ACCESS_KEY_ID="REDACTED_AWS_ACCESS_KEY"
export AWS_SECRET_ACCESS_KEY="REDACTED_AWS_SECRET_KEY"
export AWS_DEFAULT_REGION="us-west-2"          # 主区域（Lambda/DynamoDB/S3/CFN）

# 验证凭证
aws sts get-caller-identity
```

---

## 1. 证书操作（Let's Encrypt → ACM → CloudFront）

### 1.1 Let's Encrypt 签发（DNS-01 TXT，acme.sh 内置 NameSilo hook）

> 背景：NameSilo 的 CNAME **拒绝下划线开头的值**且**不支持 NS 记录**，导致 ACM DNS 验证和 Route53 委托都走不通；
> 但 **TXT 支持下划线 host**，故用 DNS-01 TXT 挑战。

```bash
# 一次性：安装 acme.sh
curl -s https://get.acme.sh | sh -s email=你的邮箱

# 签发（TXT 记录由 acme.sh 自动加到 NameSilo 并自动清理）
export Namesilo_Key=$(cat ~/.config/namesilo/api.key)
~/.acme.sh/acme.sh --issue --dns dns_namesilo -d arch.sky-city.me --server letsencrypt
# 证书输出：~/.acme.sh/arch.sky-city.me_ecc/{arch.sky-city.me.cer, arch.sky-city.me.key, ca.cer, fullchain.cer}

# 手动强制续期（验证用）
~/.acme.sh/acme.sh --renew -d arch.sky-city.me --force
```

### 1.2 导入 ACM（us-east-1）

> ⚠️ **不要用 `aws acm import-certificate --certificate file://...`**——AWS CLI 对 blob 参数有解析 bug
> （报 "certificate field contains more than one certificate" / "Invalid base64"）。用 boto3 直接传 bytes。

```bash
source ~/proxy.sh
export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...
python3 <<'EOF'
import boto3
acm = boto3.client('acm', region_name='us-east-1')   # CloudFront 证书必须 us-east-1
d = '/home/tianbao/.acme.sh/arch.sky-city.me_ecc'
arn = acm.import_certificate(
    Certificate=open(f'{d}/arch.sky-city.me.cer','rb').read(),
    PrivateKey=open(f'{d}/arch.sky-city.me.key','rb').read(),
    CertificateChain=open(f'{d}/ca.cer','rb').read(),
)['CertificateArn']
print(arn)   # 记下这个 ARN，用于 CloudFront
EOF
```

### 1.3 查看/删除证书

```bash
# 查看证书状态
aws acm describe-certificate --region us-east-1 --certificate-arn <ARN> \
  --query 'Certificate.{Status:Status,Domain:DomainName,NotAfter:NotAfter}' --output json

# 删除证书
aws acm delete-certificate --region us-east-1 --certificate-arn <ARN>

# ⚠️ 注意：list-certificates 对 imported 证书可能返回空（ACM 怪癖），用 describe 确认。
```

### 1.4 自动续期（关键）

```bash
# 一键脚本：续期后 导入ACM → 更新CloudFront → 同步CFN参数 → 清理旧证书
bash infra/renew-acm.sh

# 配置 acme.sh 续期成功后自动执行（一次性配置，cron 每 60 天自动续期）
~/.acme.sh/acme.sh --install-cert -d arch.sky-city.me \
  --key-file ~/.acme.sh/arch.sky-city.me_ecc/arch.sky-city.me.key \
  --fullchain-file ~/.acme.sh/arch.sky-city.me_ecc/fullchain.cer \
  --reloadcmd "bash /home/tianbao/tasks/person/operon/infra/renew-acm.sh"

# 确认 cron 已装
crontab -l | grep acme
```

### 1.5 NameSilo 域名解析（可选的手工操作）

```bash
# 查看 sky-city.me 全部记录
bash ~/.claude/skills/namesilo-dns/namesilo-dns.sh list --domain sky-city.me

# 添加最终 CNAME（若重建）：arch → CloudFront 域名
bash ~/.claude/skills/namesilo-dns/namesilo-dns.sh add arch CNAME d3recyygcu2a3x.cloudfront.net --domain sky-city.me

# 验证解析
dig arch.sky-city.me +short @8.8.8.8
```

---

## 2. 创建资源

### 2.1 一键部署（推荐）

```bash
cd operon && ./infra/deploy.sh dev
# 流程：确保 SSM 密钥(jwt_seed/admin_password) → musl 编译 → 打包 bootstrap.zip
#      → 上传 Lambda 代码 + 前端 → CloudFormation 部署（Lambda/URL/DynamoDB/S3/CloudFront）
```

### 2.2 手动 CloudFormation 部署（等价命令）

```bash
source ~/proxy.sh
export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... AWS_DEFAULT_REGION=us-west-2

# 编译并打包（musl 静态，兼容 Lambda AL2023）
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl -p operon-site
mkdir -p /tmp/pkg && cp target/x86_64-unknown-linux-musl/release/operon-site /tmp/pkg/bootstrap
(cd /tmp/pkg && zip -q /home/tianbao/tasks/person/operon/target/bootstrap-musl.zip bootstrap)

# 上传代码包
aws s3api put-object --bucket operon-deploy-317618187345-us-west-2 --key code/bootstrap.zip \
  --body target/bootstrap-musl.zip --cli-read-timeout 290 --cli-connect-timeout 30

# CloudFormation 部署（首次创建 / 更新）
# 静态参数集中在 infra/params.json（Project/Env/Domain/证书/内存等），CodeKey 动态指定
aws cloudformation deploy \
  --stack-name operon-dev \
  --template-file infra/template.yaml \
  --parameter-overrides \
      $(python3 -c "import json; p=json.load(open('infra/params.json')); p['CodeKey']='code/bootstrap.zip'; [print(f'{k}={v}') for k,v in p.items()]") \
  --capabilities CAPABILITY_NAMED_IAM \
  --no-fail-on-empty-changeset
# 单个参数可用环境变量覆盖（deploy.sh 支持）：OPERON_PROJECT / OPERON_ENV / OPERON_DOMAIN / ACM_ARN
```

### 2.3 核对创建结果

```bash
aws cloudformation describe-stacks --stack-name operon-dev --query 'Stacks[0].Outputs' --output table
# 输出含：CloudFrontUrl / FunctionUrl / LeadsTableName / FrontendBucket / 参数名
```

---

## 3. 更新资源

### 3.1 更新 Lambda 代码（改了 Rust 后）

```bash
# 同 2.2：重新 musl 编译 → 打包 → 上传（新 key 或覆盖）→ CFN deploy（CodeKey 变化触发更新）
# 注意：CodeKey 每次用不同的 key（如 code/$(date +%s)/bootstrap.zip）才会触发 Lambda 更新
```

### 3.2 改管理员密码

```bash
aws ssm put-parameter --name /operon/dev/admin_password --type SecureString --value 新密码 --overwrite
# 下次冷启动生效（SSM 拉取缓存）
```

### 3.3 手动切换 CloudFront 证书（续期脚本的核心逻辑）

```bash
python3 <<'EOF'
import boto3
cf = boto3.client('cloudfront', region_name='us-west-2')
d = cf.get_distribution(Id='E1NYOKR46XYB9U')
cfg = d['Distribution']['DistributionConfig']
vc = cfg['ViewerCertificate']
vc['ACMCertificateArn'] = '新证书ARN'
vc['Certificate'] = '新证书ARN'
vc.pop('IAMCertificateId', None); vc.pop('CloudFrontDefaultCertificate', None)
cf.update_distribution(Id='E1NYOKR46XYB9U', DistributionConfig=cfg, IfMatch=d['ETag'])
EOF
# 查看当前 distribution 配置
aws cloudfront get-distribution --id E1NYOKR46XYB9U --query 'Distribution.DistributionConfig.{Aliases:Aliases.Items,Cert:ViewerCertificate.ACMCertificateArn}' --output json
```

---

## 4. 测试资源

### 4.1 网站与 API（curl，走代理）

```bash
source ~/proxy.sh
BASE=https://arch.sky-city.me

# 健康检查
curl -s -w "\n[HTTP %{http_code}]\n" "$BASE/health"

# 提交采购需求（⚠️ POST 必须带 x-amz-content-sha256，否则 403）
BODY='{"name":"张伟","company":"示例科技","email":"z@test.com","requirements":"测试需求","budget":"5k-20k"}'
HASH=$(printf '%s' "$BODY" | sha256sum | cut -d' ' -f1)
curl -s -w "\n[HTTP %{http_code}]\n" -X POST \
  -H "Content-Type: application/json" -H "x-amz-content-sha256: $HASH" \
  -d "$BODY" "$BASE/api/leads"
```

### 4.2 管理员登录 + 查看需求（JWT 走 X-Authorization）

```bash
source ~/proxy.sh
BASE=https://arch.sky-city.me
PASS=$(aws ssm get-parameter --name /operon/dev/admin_password --with-decryption --query Parameter.Value --output text)
LB=$(printf '{"username":"admin","password":"%s"}' "$PASS")
LHASH=$(printf '%s' "$LB" | sha256sum | cut -d' ' -f1)
TOKEN=$(curl -s -X POST -H "Content-Type: application/json" -H "x-amz-content-sha256: $LHASH" \
  -d "$LB" "$BASE/api/admin/login" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
curl -s -H "X-Authorization: Bearer $TOKEN" "$BASE/api/admin/leads"
```

### 4.3 SigV4 直连 Function URL（模拟裸访问被拒 + 签名访问）

```bash
# 裸访问（应 403，AWS_IAM 生效）
curl -s -o /dev/null -w "%{http_code}\n" https://<url_id>.lambda-url.us-west-2.on.aws/health

# SigV4 签名访问（绕过 CloudFront）
python3 infra/sigv4_request.py https://<url_id>.lambda-url.us-west-2.on.aws/health
```

### 4.4 浏览器验证

打开 `https://arch.sky-city.me`（宣传页）、`/admin.html`（管理后台，用 SSM 里的密码登录）。

### 4.5 查日志 / 性能

```bash
# Lambda 日志（排障）
aws logs filter-log-events --log-group-name /aws/lambda/operon-dev-site \
  --start-time $(date -d '1 hour ago' +%s) --query 'events[].message' --output text

# 性能（REPORT 行含 Duration / Max Memory / INIT）
aws logs filter-log-events --log-group-name /aws/lambda/operon-dev-site \
  --query 'events[].message' --output text | grep -E 'REPORT|INIT_REPORT'
```

---

## 5. 成本与告警

### 5.1 查账单

```bash
# 本月各服务费用
aws ce get-cost-and-usage \
  --time-period Start=$(date +%Y-%m-01),End=$(date +%Y-%m-%d) \
  --granularity MONTHLY --metrics UnblendedCost \
  --group-by Type=DIMENSION,Key=SERVICE --output json
```

### 5.2 成本告警（Budgets + SNS）

```bash
# 创建 SNS Topic
aws sns create-topic --name operon-cost-alerts   # 记下 TopicArn

# 订阅邮箱（⚠️ 需去邮箱点确认链接）
aws sns subscribe --topic-arn <TopicArn> --protocol email \
  --notification-endpoint 18217401108@163.com

# 创建预算（$10/月，>85% 和 >100% 通知）
aws budgets create-budget \
  --account-id $(aws sts get-caller-identity --query Account --output text) \
  --budget '{"BudgetLimit":{"Amount":"10","Unit":"USD"},"TimeUnit":"MONTHLY","BudgetType":"COST","BudgetName":"operon-monthly-budget"}' \
  --notifications-with-subscribers '[{"Notification":{"NotificationType":"ACTUAL","ComparisonOperator":"GREATER_THAN","Threshold":85,"ThresholdType":"PERCENTAGE"},"Subscribers":[{"SubscriptionType":"SNS","Address":"<TopicArn>"}]},{"Notification":{"NotificationType":"ACTUAL","ComparisonOperator":"GREATER_THAN","Threshold":100,"ThresholdType":"PERCENTAGE"},"Subscribers":[{"SubscriptionType":"SNS","Address":"<TopicArn>"}]}]'

# 发测试通知
aws sns publish --topic-arn <TopicArn> --subject "测试" --message "测试告警链路"
```

### 5.3 ECR / 资源清理

```bash
# 列出 ECR 仓库（历史遗留可能产生小额费用）
aws ecr describe-repositories --region us-west-2
# 删除不再需要的仓库（--force 连同镜像）
aws ecr delete-repository --repository-name pcc-service --force --region us-west-2
```

---

## 6. 彻底清理（停止计费）

```bash
# 删除 CloudFormation 栈（Lambda/DynamoDB/S3 前端/CloudFront/权限 全部移除）
aws cloudformation delete-stack --stack-name operon-dev

# 删除部署桶
aws s3 rb --force s3://operon-deploy-317618187345-us-west-2

# 删除 SSM 参数（可选）
aws ssm delete-parameter --name /operon/dev/jwt_seed
aws ssm delete-parameter --name /operon/dev/admin_password

# 删除 SNS / 预算（可选）
aws budgets delete-budget --account-id $(aws sts get-caller-identity --query Account --output text) --budget-name operon-monthly-budget
aws sns delete-topic --topic-arn <TopicArn>

# 删除 ACM 证书（可选）
aws acm delete-certificate --region us-east-1 --certificate-arn <ARN>

# 域名（NameSilo）与 acme.sh 证书目录保留则无成本
```

---

## 附：环境细节速查

| 项 | 值 |
|---|---|
| 账号 | 317618187345（IAM user `silas`） |
| 区域 | us-west-2（主）/ us-east-1（ACM 证书） |
| CloudFront | E1NYOKR46XYB9U（域名 arch.sky-city.me） |
| Lambda | operon-dev-site（x86_64, 256MB, provided.al2023 + Web Adapter） |
| DynamoDB | operon-dev-leads |
| S3 前端 | operon-dev-frontend |
| SSM | /operon/dev/{jwt_seed, admin_password, acm_cert_arn} |
| 证书目录 | ~/.acme.sh/arch.sky-city.me_ecc/ |
| 续期脚本 | infra/renew-acm.sh（含 boto3 依赖） |
| 代理 | ~/proxy.sh（必须） |

#!/usr/bin/env bash
# Let's Encrypt 证书续期后的完整处理：
#   导入 ACM（us-east-1）→ 更新 CloudFront ViewerCertificate → 同步 CFN 栈参数 → 清理旧证书
#
# 由 acme.sh 在续期成功后自动调用（reloadcmd / renew-hook）：
#   acme.sh --install-cert -d <DOMAIN> --reloadcmd "bash /path/to/renew-acm.sh"
#
# 依赖：boto3（pip install boto3），走代理 ~/proxy.sh，测试账号凭证在下方可覆盖。

set -euo pipefail

source ~/proxy.sh
export AWS_ACCESS_KEY_ID="${AWS_ACCESS_KEY_ID:-REDACTED_AWS_ACCESS_KEY}"
export AWS_SECRET_ACCESS_KEY="${AWS_SECRET_ACCESS_KEY:-REDACTED_AWS_SECRET_KEY}"
export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-us-west-2}"   # aws CLI 命令默认区域（SSM/CFN 在 us-west-2）

DOMAIN="${DOMAIN:-arch.sky-city.me}"
DIST_ID="${DIST_ID:-E1NYOKR46XYB9U}"
STACK="${STACK:-operon-dev}"
REGION_ACM="us-east-1"          # CloudFront 证书必须 us-east-1
REGION_CF="us-west-2"
CERT_DIR="$HOME/.acme.sh/${DOMAIN}_ecc"
SSM_CERT_ARN="/operon/dev/acm_cert_arn"

log() { echo "[renew-acm] $*"; }

if [ ! -f "$CERT_DIR/$DOMAIN.cer" ]; then
  log "证书目录不存在: $CERT_DIR（先签发证书）"; exit 1
fi

# ---------- 1. 导入 ACM（boto3 直接传 bytes，绕开 AWS CLI 的 blob 解析 bug） ----------
log "导入 ACM..."
NEW_ARN=$(AWS_ACCESS_KEY_ID="$AWS_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$AWS_SECRET_ACCESS_KEY" python3 <<EOF
import boto3
acm = boto3.client('acm', region_name='$REGION_ACM')
d = '$CERT_DIR'
arn = acm.import_certificate(
    Certificate=open(f'{d}/$DOMAIN.cer', 'rb').read(),
    PrivateKey=open(f'{d}/$DOMAIN.key', 'rb').read(),
    CertificateChain=open(f'{d}/ca.cer', 'rb').read(),
)['CertificateArn']
print(arn)
EOF
)
log "新证书 ARN: $NEW_ARN"

# 存到 SSM，方便查当前用的证书
aws ssm put-parameter --name "$SSM_CERT_ARN" --type String --value "$NEW_ARN" --overwrite >/dev/null

# ---------- 2. 更新 CloudFront ViewerCertificate（快速生效，秒级） ----------
log "更新 CloudFront ViewerCertificate..."
AWS_ACCESS_KEY_ID="$AWS_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$AWS_SECRET_ACCESS_KEY" python3 <<EOF
import boto3
cf = boto3.client('cloudfront', region_name='$REGION_CF')
d = cf.get_distribution(Id='$DIST_ID')
cfg = d['Distribution']['DistributionConfig']
vc = cfg['ViewerCertificate']
vc['ACMCertificateArn'] = '$NEW_ARN'
vc['Certificate'] = '$NEW_ARN'
vc.pop('IAMCertificateId', None)
vc.pop('CloudFrontDefaultCertificate', None)
resp = cf.update_distribution(Id='$DIST_ID', DistributionConfig=cfg, IfMatch=d['ETag'])
print("distribution status:", resp['Distribution']['Status'])
EOF

# ---------- 3. 同步 CFN 栈参数（避免下次 cloudformation deploy 把证书回滚成旧 ARN） ----------
log "同步 CFN 参数 AcmCertificateArn..."
aws cloudformation update-stack --stack-name "$STACK" \
  --use-previous-template \
  --parameters \
    ParameterKey=Project,UsePreviousValue=true \
    ParameterKey=Environment,UsePreviousValue=true \
    ParameterKey=LambdaArchitecture,UsePreviousValue=true \
    ParameterKey=LambdaMemoryMB,UsePreviousValue=true \
    ParameterKey=CodeBucket,UsePreviousValue=true \
    ParameterKey=CodeKey,UsePreviousValue=true \
    ParameterKey=WebAdapterLayerArn,UsePreviousValue=true \
    ParameterKey=DomainName,UsePreviousValue=true \
    ParameterKey=AcmCertificateArn,ParameterValue="$NEW_ARN" \
  --capabilities CAPABILITY_NAMED_IAM >/dev/null

# ---------- 4. 清理旧 ACM 证书（用历史记录跟踪，因 list-certificates 对 imported 证书不显示） ----------
HIST_FILE="$HOME/.acme.sh/${DOMAIN}_renew_history"
echo "$NEW_ARN" >> "$HIST_FILE"    # 记录本次导入的 ARN
log "清理旧 ACM 证书（历史: $(wc -l < "$HIST_FILE") 条）..."
AWS_ACCESS_KEY_ID="$AWS_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$AWS_SECRET_ACCESS_KEY" python3 <<EOF
import boto3
acm = boto3.client('acm', region_name='$REGION_ACM')
hist = open('$HIST_FILE').read().split()
# 保留最新的 1 个，删除其余（先 describe 确认存在再删，避免误删当前在用的）
for arn in hist[:-1]:
    try:
        acm.describe_certificate(CertificateArn=arn)   # 不存在会抛异常
        acm.delete_certificate(CertificateArn=arn)
        print("deleted:", arn)
    except Exception as e:
        print("skip(not found)", arn)
# 历史只保留最新
open('$HIST_FILE', 'w').write(hist[-1] + '\n')
EOF

log "完成。当前证书: $NEW_ARN"

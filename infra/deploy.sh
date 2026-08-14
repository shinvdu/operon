#!/usr/bin/env bash
# Operon Cloud 网站端到端部署
#
# 用法: ./deploy.sh [environment]   (默认 dev)
# 流程: 确保 SSM 密钥(jwt_seed + admin_password) → 编译 site → 打包 bootstrap.zip
#       → 上传 Lambda 代码 → 上传前端(S3) → CloudFormation deploy
set -euo pipefail

# ---- 环境（代理 + 凭证） ----
source ~/proxy.sh
export AWS_ACCESS_KEY_ID="${AWS_ACCESS_KEY_ID:-REDACTED_AWS_ACCESS_KEY}"
export AWS_SECRET_ACCESS_KEY="${AWS_SECRET_ACCESS_KEY:-REDACTED_AWS_SECRET_KEY}"
export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-us-west-2}"

PROJECT="${OPERON_PROJECT:-operon}"
ENV="${1:-dev}"
REGION="${AWS_DEFAULT_REGION}"
ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
STACK_NAME="${PROJECT}-${ENV}"
BUCKET="operon-deploy-${ACCOUNT_ID}-${REGION}"
FRONTEND_BUCKET="${PROJECT}-${ENV}-frontend"
SECRETS_PATH="/${PROJECT}/${ENV}/"
PACKAGE="operon-site"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

step() { echo -e "\n====> $1"; }

step "[1/7] 确保 SSM 密钥存在（SecureString）"
# JWT seed（Ed25519）
if aws ssm get-parameter --name "${SECRETS_PATH}jwt_seed" --with-decryption --query Parameter.Value --output text >/dev/null 2>&1; then
  echo "      jwt_seed 已存在"
else
  SEED="$(cargo run -q -p operon-cli -- gen-seed)"
  aws ssm put-parameter --name "${SECRETS_PATH}jwt_seed" --type SecureString --value "$SEED" >/dev/null
  echo "      jwt_seed 已创建"
fi
# 管理员密码
if aws ssm get-parameter --name "${SECRETS_PATH}admin_password" --with-decryption --query Parameter.Value --output text >/dev/null 2>&1; then
  echo "      admin_password 已存在"
else
  PASS="$(cargo run -q -p operon-cli -- gen-seed)"
  aws ssm put-parameter --name "${SECRETS_PATH}admin_password" --type SecureString --value "$PASS" >/dev/null
  echo "      admin_password 已创建：$PASS  （请保存！登录 /admin.html 用）"
fi

step "[2/7] 编译 release（$PACKAGE，musl 静态链接）"
rustup target add x86_64-unknown-linux-musl >/dev/null 2>&1 || true
cargo build --release --target x86_64-unknown-linux-musl -p "$PACKAGE"

step "[3/7] 打包 bootstrap.zip"
ZIP="$ROOT/target/bootstrap-musl.zip"
rm -f "$ZIP"
PKG_DIR="$(mktemp -d)"
cp "$ROOT/target/x86_64-unknown-linux-musl/release/${PACKAGE}" "$PKG_DIR/bootstrap"
( cd "$PKG_DIR" && zip -q "$ZIP" bootstrap )
rm -rf "$PKG_DIR"
ls -lh "$ZIP"

step "[4/7] 上传 Lambda 代码"
if ! aws s3api head-bucket --bucket "$BUCKET" >/dev/null 2>&1; then
  aws s3api create-bucket --bucket "$BUCKET" --region "$REGION" \
    --create-bucket-configuration LocationConstraint="$REGION" >/dev/null
fi
CODE_KEY="code/$(date +%s)/bootstrap.zip"
aws s3api put-object --bucket "$BUCKET" --key "$CODE_KEY" --body "$ZIP" \
  --cli-read-timeout 290 --cli-connect-timeout 30 >/dev/null
echo "      已上传 $CODE_KEY"

step "[5/7] CloudFormation 部署 $STACK_NAME（先建前端桶，再传文件）"
# 从独立参数文件 infra/params.json 读取参数（CodeKey 动态生成；可用环境变量覆盖单个参数）
PARAM_OVERRIDES=$(python3 - "$CODE_KEY" <<'PYEOF'
import json, os, sys
code_key = sys.argv[1]
p = json.load(open('infra/params.json'))
p['CodeKey'] = code_key
# 环境变量覆盖（可选）：OPERON_PROJECT / OPERON_ENV / OPERON_DOMAIN / ACM_ARN
for k, env in {'Project':'OPERON_PROJECT','Environment':'OPERON_ENV','DomainName':'OPERON_DOMAIN','AcmCertificateArn':'ACM_ARN'}.items():
    if os.environ.get(env):
        p[k] = os.environ[env]
for k, v in p.items():
    print(f"{k}={v}")
PYEOF
)
aws cloudformation deploy \
  --stack-name "$STACK_NAME" \
  --template-file infra/template.yaml \
  --parameter-overrides $PARAM_OVERRIDES \
  --capabilities CAPABILITY_NAMED_IAM \
  --no-fail-on-empty-changeset

step "[6/7] 上传前端静态文件到 s3://${FRONTEND_BUCKET}"
for f in index.html admin.html; do
  if [ -f "$ROOT/frontend/$f" ]; then
    aws s3api put-object --bucket "$FRONTEND_BUCKET" --key "$f" \
      --body "$ROOT/frontend/$f" --content-type "text/html; charset=utf-8" >/dev/null
    echo "      已上传 $f"
  else
    echo "      跳过（缺少 $f）"
  fi
done

step "[7/7] 完成"
aws cloudformation describe-stacks --stack-name "$STACK_NAME" \
  --query 'Stacks[0].Outputs' --output table
echo
echo "网站:  $(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --query "Stacks[0].Outputs[?OutputKey=='CloudFrontUrl'].OutputValue" --output text)"
echo "管理:  $(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --query "Stacks[0].Outputs[?OutputKey=='CloudFrontUrl'].OutputValue" --output text)/admin.html"

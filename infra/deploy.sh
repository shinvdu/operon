#!/usr/bin/env bash
# operon 端到端部署脚本
#
# 用法: ./deploy.sh [environment]   (默认 dev)
# 流程: 确保 SSM 密钥 → 编译 release → 打包 bootstrap.zip → 上传 S3 →
#       CloudFormation deploy（Lambda + Function URL + DynamoDB + CloudFront OAC）
#
# 对应文章第五节的 `operon deploy` CLI：一行命令把整套基础设施从无到有。
set -euo pipefail

# ---- 环境（代理 + 凭证；优先取已 export 的环境变量，否则用测试账号默认值） ----
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
SECRETS_PATH="/${PROJECT}/${ENV}/"
SSM_KEY="${SECRETS_PATH}jwt_seed"
PACKAGE="operon-example"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

step() { echo -e "\n====> $1"; }

step "[1/6] 确保 SSM JWT 密钥存在（SecureString，KMS 加密）"
if aws ssm get-parameter --name "$SSM_KEY" --with-decryption --query Parameter.Value --output text >/dev/null 2>&1; then
  echo "      已存在: $SSM_KEY"
else
  SEED="$(cargo run -q -p operon-cli -- gen-seed)"
  aws ssm put-parameter --name "$SSM_KEY" --type SecureString --value "$SEED" >/dev/null
  echo "      已创建: $SSM_KEY"
fi

step "[2/6] 编译 release（$PACKAGE，musl 静态链接，兼容 Lambda AL2023）"
rustup target add x86_64-unknown-linux-musl >/dev/null 2>&1 || true
cargo build --release --target x86_64-unknown-linux-musl -p "$PACKAGE"

step "[3/6] 打包 bootstrap.zip（Web Adapter 入口约定为 bootstrap）"
ZIP="$ROOT/target/bootstrap-musl.zip"
rm -f "$ZIP"
PKG_DIR="$(mktemp -d)"
cp "$ROOT/target/x86_64-unknown-linux-musl/release/${PACKAGE}" "$PKG_DIR/bootstrap"
( cd "$PKG_DIR" && zip -q "$ZIP" bootstrap )
rm -rf "$PKG_DIR"
ls -lh "$ZIP"

step "[4/6] 确保部署 bucket 并上传代码"
if ! aws s3api head-bucket --bucket "$BUCKET" >/dev/null 2>&1; then
  aws s3api create-bucket --bucket "$BUCKET" --region "$REGION" \
    --create-bucket-configuration LocationConstraint="$REGION" >/dev/null
  echo "      已创建 s3://${BUCKET}"
else
  echo "      已存在 s3://${BUCKET}"
fi
CODE_KEY="code/$(date +%s)/bootstrap.zip"
# 环境带宽较慢（~50KB/s），用长超时上传；put-object 优于 s3 cp（无进度线程挂起问题）
aws s3api put-object --bucket "$BUCKET" --key "$CODE_KEY" --body "$ZIP" \
  --cli-read-timeout 290 --cli-connect-timeout 30 >/dev/null
echo "      已上传 $CODE_KEY"

step "[5/6] CloudFormation 部署 $STACK_NAME（CloudFront 创建需几分钟）"
aws cloudformation deploy \
  --stack-name "$STACK_NAME" \
  --template-file infra/template.yaml \
  --parameter-overrides \
      Project="$PROJECT" \
      Environment="$ENV" \
      CodeBucket="$BUCKET" \
      CodeKey="$CODE_KEY" \
  --capabilities CAPABILITY_NAMED_IAM \
  --no-fail-on-empty-changeset

step "[6/6] 部署完成，输出端点"
aws cloudformation describe-stacks --stack-name "$STACK_NAME" \
  --query 'Stacks[0].Outputs' --output table
echo
echo "提示:"
echo "  1) CloudFront 就绪后直接 curl:  $(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --query "Stacks[0].Outputs[?OutputKey=='CloudFrontUrl'].OutputValue" --output text)"
echo "  2) Function URL 需 SigV4 签名:  python3 infra/sigv4_request.py <FunctionUrl>/health"
echo "  3) 签发测试 JWT:  cargo run -q -p operon-cli -- token --seed <seed> --sub user-123 --email test@example.com"

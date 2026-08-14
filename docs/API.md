# operon API 文档（含 curl 示例）

> 当前骨架实际部署的路由 + 规划中的 API。所有 curl 示例均经过端到端实测。
> **Base URL（当前部署）**：`https://d3recyygcu2a3x.cloudfront.net`
> 获取当前 URL：`aws cloudformation describe-stacks --stack-name operon-dev --query "Stacks[0].Outputs[?OutputKey=='CloudFrontUrl'].OutputValue" --output text`

## 通用约定

- **认证头**：业务 JWT 一律走 **`X-Authorization: Bearer <jwt>`**。
  （CloudFront OAC 会占用标准 `Authorization` 头做 SigV4 签名，传过去的业务 JWT 到不了应用。）
- **POST/PUT 必须带 `x-amz-content-sha256`**：Lambda Function URL 不支持 unsigned payload。
- **网络**：本机访问走代理 `source ~/proxy.sh`（直连极慢）。
- **错误格式**（统一）：`{"error":{"code":"<CODE>","message":"<原因>"}}`。
- **返回码**：200/201 成功；400 参数错；401 未认证/token 无效；403 签名校验失败；500 内部错误。

---

## 一、已实现 API

### 1. `GET /health` — 健康检查

无需认证。用于探测服务存活与链路（CloudFront → OAC → Lambda）。

```bash
source ~/proxy.sh
curl https://d3recyygcu2a3x.cloudfront.net/health
```

**响应 200**
```json
{"status":"ok"}
```

---

### 2. `POST /users` — 创建用户

无需认证。将用户写入 DynamoDB（`operon-dev-users`，pk=`USERS`）。

**请求体**
```json
{"email":"alice@example.com","name":"Alice"}
```

**curl**
```bash
source ~/proxy.sh
BODY='{"email":"alice@example.com","name":"Alice"}'
HASH=$(printf '%s' "$BODY" | sha256sum | cut -d' ' -f1)
curl -X POST \
  -H "Content-Type: application/json" \
  -H "x-amz-content-sha256: $HASH" \
  -d "$BODY" \
  https://d3recyygcu2a3x.cloudfront.net/users
```

**响应 201**
```json
{"user_id":"a512311b-237a-4f9a-a87b-3dad515e791d","email":"alice@example.com","name":"Alice"}
```

**常见错误**
| 场景 | 状态码 | 响应 |
|---|---|---|
| 忘了 `x-amz-content-sha256` | 403 | `{"message":"The request signature we calculated does not match..."}` |
| body 不是合法 JSON / 缺字段 | 400 | axum 反序列化拒绝 |

---

### 3. `GET /users` — 列出所有用户

无需认证。查询 DynamoDB 中 pk=`USERS` 的所有记录。

```bash
source ~/proxy.sh
curl https://d3recyygcu2a3x.cloudfront.net/users
```

**响应 200**
```json
[
  {"user_id":"048147dd-...","email":"bob@example.com","name":"Bob"},
  {"user_id":"a512311b-...","email":"alice@example.com","name":"Alice"}
]
```

---

### 4. `GET /me` — 当前用户（JWT 保护）

**必需认证**：`X-Authorization: Bearer <jwt>`。返回 JWT 声明的身份信息，演示 Ed25519 无状态验证。

**先签发一个测试 JWT**（用 SSM 里的 seed）：
```bash
source ~/proxy.sh
SEED=$(aws ssm get-parameter --name /operon/dev/jwt_seed --with-decryption --query Parameter.Value --output text)
TOKEN=$(cargo run -q -p operon-cli -- token --seed "$SEED" --sub user-123 --email alice@example.com)
```

**请求**
```bash
curl -H "X-Authorization: Bearer $TOKEN" \
  https://d3recyygcu2a3x.cloudfront.net/me
```

**响应 200**
```json
{"email":"alice@example.com","exp":1786772146,"sub":"user-123"}
```

**常见错误**
| 场景 | 状态码 | 响应 |
|---|---|---|
| 无 `X-Authorization` 头 | 401 | `{"error":{"code":"UNAUTHORIZED","message":"missing Authorization header"}}` |
| token 格式错误（`not.a.jwt`） | 401 | `... "bad signature encoding"` |
| 签名被篡改 | 401 | `... "bad signature length"` |
| token 过期（exp < now） | 401 | `... "token expired"` |

> ⚠️ 若误把 JWT 放 `Authorization: Bearer`，会被 CloudFront OAC 替换成 SigV4 签名，应用收不到 → 也返回 401 missing。

---

## 二、规划中 API（设计已定，待实现）

以下路由在 `DESIGN.md §2.1.6` 已定义，实现 OIDC 后自动挂载：

| 方法/路径 | 说明 |
|---|---|
| `GET /.well-known/jwks.json` | Ed25519 公钥端点（供第三方验证框架签发的 JWT） |
| `GET /api/auth/{provider}` | 发起第三方登录（302 跳转到 Google/GitHub，带 PKCE） |
| `GET /api/auth/{provider}/callback` | 登录回调：换 token → 验 ID Token → 签发自有 JWT → 写 cookie |

对应框架调用（`DESIGN.md`）：
```rust
let oidc = OidcRouter::builder()
    .base_url("https://myapp.example.com")
    .route_prefix("/api/auth")
    .cookie_key(key).token_signer(signer)
    .provider(google_config, client_id, client_secret, MyAuthHandler)
    .build().await?;
```

---

## 三、附：SigV4 直连 Function URL

Function URL 配置为 `AWS_IAM`，公网裸访问返回 403。绕过 CloudFront 直连需 SigV4 签名
（用仓库自带工具）：

```bash
source ~/proxy.sh
export AWS_ACCESS_KEY_ID=REDACTED_AWS_ACCESS_KEY AWS_SECRET_ACCESS_KEY=... AWS_DEFAULT_REGION=us-west-2
cd infra
python3 sigv4_request.py https://jyyk44xtwr5oriprkrpnlkb56i0bcsny.lambda-url.us-west-2.on.aws/health
# 带 JWT：
python3 sigv4_request.py -H "X-Authorization: Bearer $TOKEN" \
  https://jyyk44xtwr5oriprkrpnlkb56i0bcsny.lambda-url.us-west-2.on.aws/me
```

**裸访问（无签名）验证安全性**
```bash
curl https://jyyk44xtwr5oriprkrpnlkb56i0bcsny.lambda-url.us-west-2.on.aws/health
# → 403 Forbidden（AWS_IAM 生效，只有签名/CloudFront OAC 能进）
```

---

## 四、错误码速查

| CODE | HTTP | 含义 |
|---|---|---|
| `UNAUTHORIZED` | 401 | 缺 token / token 无效 / 过期 |
| `BAD_REQUEST` | 400 | 参数或请求体不合法 |
| `NOT_FOUND` | 404 | 资源不存在（DynamoDB 未命中） |
| `CONFLICT` | 409 | 条件冲突 |
| `INTERNAL` | 500 | 内部错误（数据库/AWS 调用失败） |
| （AWS 层） | 403 | SigV4 签名校验失败（缺 hash / 裸访问） |

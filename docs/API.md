# Operon Cloud 网站 API 文档（含 curl 示例）

> 当前线上应用 `operon-site`（单 Lambda 承载全部 API）。所有示例均端到端实测。
> **Base URL**：`https://d3recyygcu2a3x.cloudfront.net`

## 通用约定

- **认证头**：业务 JWT 走 **`X-Authorization: Bearer <jwt>`**（CloudFront OAC 占用标准 `Authorization` 头）。
- **POST 必须带 `x-amz-content-sha256`**：Lambda Function URL 不支持 unsigned payload（前端已用 `crypto.subtle` 自动计算）。
- **网络**：本机访问走代理 `source ~/proxy.sh`。
- **错误格式**：`{"error":{"code":"<CODE>","message":"<原因>"}}`。

---

## 1. `GET /health` — 健康检查

```bash
source ~/proxy.sh
curl https://d3recyygcu2a3x.cloudfront.net/health
```
**200** `{"service":"operon-site","status":"ok"}`

---

## 2. `POST /api/leads` — 提交采购需求（公开）

公司宣传页的采购需求表单调用的接口，写入 DynamoDB `operon-{env}-leads`。

**请求体**
```json
{
  "name": "张伟",
  "company": "云上科技有限公司",
  "email": "zhangwei@test.com",
  "phone": "13900000000",
  "requirements": "需要一套无服务器架构的企业官网",
  "budget": "20k-100k"
}
```

**curl**
```bash
source ~/proxy.sh
BODY='{"name":"张伟","company":"云上科技有限公司","email":"z@test.com","requirements":"企业官网和后台","budget":"5k-20k"}'
HASH=$(printf '%s' "$BODY" | sha256sum | cut -d' ' -f1)
curl -X POST -H "Content-Type: application/json" -H "x-amz-content-sha256: $HASH" \
  -d "$BODY" https://d3recyygcu2a3x.cloudfront.net/api/leads
```
**201**
```json
{"id":"c1d2e51b-...","name":"张伟","company":"云上科技有限公司","email":"z@test.com","phone":null,"requirements":"企业官网和后台","budget":"5k-20k","status":"new","created_at":1786701600}
```
**403**（忘带 hash）→ AWS SignatureDoesNotMatch；**400**（非法 JSON）→ BadRequest。

---

## 3. `POST /api/admin/login` — 管理员登录

内置账号 `admin`，密码存 SSM（SecureString）。成功签发 8 小时有效 JWT（含 `role: admin`）。

```bash
source ~/proxy.sh
PASS=$(aws ssm get-parameter --name /operon/dev/admin_password --with-decryption --query Parameter.Value --output text)
BODY=$(printf '{"username":"admin","password":"%s"}' "$PASS")
HASH=$(printf '%s' "$BODY" | sha256sum | cut -d' ' -f1)
curl -X POST -H "Content-Type: application/json" -H "x-amz-content-sha256: $HASH" \
  -d "$BODY" https://d3recyygcu2a3x.cloudfront.net/api/admin/login
```
**200** `{"token":"eyJ...","username":"admin"}`
**401**（密码错误）`{"error":{"code":"UNAUTHORIZED","message":"invalid credentials"}}`

---

## 4. `GET /api/admin/leads` — 需求列表（管理员）

JWT 保护 + `role: admin` 校验，返回按时间倒序的采购需求。

```bash
source ~/proxy.sh
TOKEN=<登录返回的 token>
curl -H "X-Authorization: Bearer $TOKEN" \
  https://d3recyygcu2a3x.cloudfront.net/api/admin/leads
```
**200**
```json
[
  {"id":"c1d2e51b-...","name":"张伟","company":"云上科技有限公司","email":"z@test.com","phone":"13900000000","requirements":"...","budget":"20k-100k","status":"new","created_at":1786701600}
]
```
| 场景 | 状态 | 响应 |
|---|---|---|
| 无 token | 401 | `missing Authorization header` |
| 非 admin 的 JWT | 403 | `admin only` |
| 篡改/过期 token | 401 | `bad signature` / `token expired` |

---

## 错误码速查

| CODE | HTTP | 含义 |
|---|---|---|
| `UNAUTHORIZED` | 401 | 缺 token / 密码错 / token 无效 |
| `FORBIDDEN` | 403 | 非 admin 访问管理接口 |
| `BAD_REQUEST` | 400 | 参数或请求体不合法 |
| `INTERNAL` | 500 | 内部错误（数据库等） |
| （AWS 层） | 403 | 缺 `x-amz-content-sha256` / 裸访问 Function URL |

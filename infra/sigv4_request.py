#!/usr/bin/env python3
"""向 Lambda Function URL（AuthType=AWS_IAM）发起 SigV4 签名请求。

用法:
  AWS_ACCESS_KEY_ID=xxx AWS_SECRET_ACCESS_KEY=yyy python3 sigv4_request.py \
      https://<id>.lambda-url.us-west-2.on.aws/health
  ... -X POST -d '{"email":"a@b.com","name":"Alice"}' <url>/users
  ... -H 'Authorization: Bearer <jwt>' <url>/me

说明: Function URL 用 AWS_IAM 认证时公网不能直接访问（对应文章「比裸奔的
API Gateway 端点还安全」），只有带 SigV4 签名（或经 CloudFront OAC 转发）的
请求才能到达 Lambda。
"""
import argparse
import datetime
import hashlib
import hmac
import os
import sys
import urllib.error
import urllib.parse
import urllib.request


def sign(key: bytes, msg: str) -> bytes:
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def get_signature_key(secret: str, date_stamp: str, region: str, service: str) -> bytes:
    k = sign(("AWS4" + secret).encode(), date_stamp)
    k = sign(k, region)
    k = sign(k, service)
    return sign(k, "aws4_request")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("url")
    ap.add_argument("-X", dest="method", default="GET")
    ap.add_argument("-d", dest="body", default=None, help="请求体（字符串，可为 JSON）")
    ap.add_argument("-H", dest="headers", action="append", default=[], help="附加请求头 'Name: value'")
    ap.add_argument("--region", default="us-west-2")
    ap.add_argument("--service", default="lambda")
    args = ap.parse_args()

    access = os.environ["AWS_ACCESS_KEY_ID"]
    secret = os.environ["AWS_SECRET_ACCESS_KEY"]
    region, service = args.region, args.service
    method = args.method.upper()
    body = args.body or ""
    payload_hash = hashlib.sha256(body.encode()).hexdigest()

    parsed = urllib.parse.urlparse(args.url)
    host = parsed.netloc
    canonical_uri = parsed.path or "/"
    canonical_qs = parsed.query

    amz_date = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    date_stamp = amz_date[:8]

    headers = {
        "host": host,
        "x-amz-date": amz_date,
        "x-amz-content-sha256": payload_hash,
    }
    for h in args.headers:
        name, _, val = h.partition(":")
        headers[name.strip().lower()] = val.strip()

    sorted_headers = sorted(headers.items())
    canonical_headers = "".join(f"{k}:{v}\n" for k, v in sorted_headers)
    signed_headers = ";".join(k for k, _ in sorted_headers)

    canonical_request = "\n".join(
        [method, canonical_uri, canonical_qs, canonical_headers, signed_headers, payload_hash]
    )
    scope = f"{date_stamp}/{region}/{service}/aws4_request"
    string_to_sign = "\n".join(
        [
            "AWS4-HMAC-SHA256",
            amz_date,
            scope,
            hashlib.sha256(canonical_request.encode()).hexdigest(),
        ]
    )
    sig = hmac.new(
        get_signature_key(secret, date_stamp, region, service),
        string_to_sign.encode(),
        hashlib.sha256,
    ).hexdigest()
    auth = (
        f"AWS4-HMAC-SHA256 Credential={access}/{scope}, "
        f"SignedHeaders={signed_headers}, Signature={sig}"
    )

    req = urllib.request.Request(args.url, method=method, data=body.encode() if body else None)
    for k, v in headers.items():
        req.add_header(k, v)
    req.add_header("Authorization", auth)

    try:
        resp = urllib.request.urlopen(req, timeout=60)
        print(f"[{resp.status}]")
        print(resp.read().decode())
        return 0
    except urllib.error.HTTPError as e:
        print(f"[{e.code}]")
        print(e.read().decode())
        return 1


if __name__ == "__main__":
    sys.exit(main())

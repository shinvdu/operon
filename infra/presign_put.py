#!/usr/bin/env python3
"""生成 S3 PUT presigned URL（标准库实现 SigV4 presign）。

用法: AWS_ACCESS_KEY_ID=.. AWS_SECRET_ACCESS_KEY=.. python3 presign_put.py <bucket> <key>
然后:  curl -T <file> "$URL" -H "Expect:"
"""
import datetime
import hashlib
import hmac
import os
import sys
import urllib.parse


def sign(key: bytes, msg: str) -> bytes:
    return hmac.new(key, msg.encode(), hashlib.sha256).digest()


def get_sig_key(secret: str, date_stamp: str, region: str, service: str) -> bytes:
    k = sign(("AWS4" + secret).encode(), date_stamp)
    k = sign(k, region)
    k = sign(k, service)
    return sign(k, "aws4_request")


def main() -> int:
    bucket, key = sys.argv[1], sys.argv[2]
    region = os.environ.get("AWS_DEFAULT_REGION", "us-west-2")
    service = "s3"
    access = os.environ["AWS_ACCESS_KEY_ID"]
    secret = os.environ["AWS_SECRET_ACCESS_KEY"]

    host = f"{bucket}.s3.{region}.amazonaws.com"
    amz_date = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    date_stamp = amz_date[:8]

    params = {
        "X-Amz-Algorithm": "AWS4-HMAC-SHA256",
        "X-Amz-Credential": f"{access}/{date_stamp}/{region}/{service}/aws4_request",
        "X-Amz-Date": amz_date,
        "X-Amz-Expires": "600",
        "X-Amz-SignedHeaders": "host",
    }
    canonical_qs = "&".join(
        f"{urllib.parse.quote(k, safe='')}={urllib.parse.quote(v, safe='')}"
        for k, v in sorted(params.items())
    )
    canonical_request = "\n".join(
        [
            "PUT",
            f"/{key}",
            canonical_qs,
            f"host:{host}",
            "host",
            "UNSIGNED-PAYLOAD",
        ]
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
        get_sig_key(secret, date_stamp, region, service),
        string_to_sign.encode(),
        hashlib.sha256,
    ).hexdigest()
    print(f"https://{host}/{key}?{canonical_qs}&X-Amz-Signature={sig}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

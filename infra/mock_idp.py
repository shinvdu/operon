#!/usr/bin/env python3
"""本地模拟 OIDC IdP —— 用于测试 operon 的 OIDC 流程（不依赖真实 Google/GitHub）。

端点：
  GET  /.well-known/openid-configuration   discovery
  GET  /authorize                           校验 PKCE challenge，302 回 redirect_uri?code=..&state=..
  POST /token                               校验 code_verifier（PKCE S256），签发 RS256 ID Token
  GET  /jwks                                RSA 公钥（JWK）

用法：python3 mock_idp.py   （监听 127.0.0.1:9090）
"""
import base64
import hashlib
import json
import secrets
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa

ISSUER = "http://127.0.0.1:9090"
PORT = 9090

# --- RSA 密钥（RS256 签 ID Token） ---
key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
pub_numbers = key.public_key().public_numbers()


def b64(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def jwks_dict():
    e = pub_numbers.e.to_bytes((pub_numbers.e.bit_length() + 7) // 8, "big")
    n = pub_numbers.n.to_bytes((pub_numbers.n.bit_length() + 7) // 8, "big")
    return {
        "keys": [
            {
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": "mock",
                "e": b64(e),
                "n": b64(n),
            }
        ]
    }


def sign_id_token(sub, aud, nonce):
    header = {"alg": "RS256", "typ": "JWT", "kid": "mock"}
    now = int(time.time())
    claims = {
        "sub": sub,
        "iss": ISSUER,
        "aud": aud,
        "exp": now + 3600,
        "iat": now,
        "email": "mock.user@example.com",
        "name": "Mock User",
        "nonce": nonce,
    }
    h = b64(json.dumps(header).encode())
    c = b64(json.dumps(claims).encode())
    signing_input = f"{h}.{c}".encode()
    sig = key.sign(signing_input, padding.PKCS1v15(), hashes.SHA256())
    return f"{h}.{c}.{b64(sig)}"


# 内存：code -> {code_challenge, nonce, redirect_uri}
codes = {}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print(f"[mock] {self.command} {self.path}", file=__import__('sys').stderr)

    def _send(self, code, body, ctype="application/json", headers=None):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if body:
            self.wfile.write(body if isinstance(body, bytes) else body.encode())

    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        if path == "/.well-known/openid-configuration":
            self._send(200, json.dumps({
                "issuer": ISSUER,
                "authorization_endpoint": f"{ISSUER}/authorize",
                "token_endpoint": f"{ISSUER}/token",
                "jwks_uri": f"{ISSUER}/jwks",
                "response_types_supported": ["code"],
                "id_token_signing_alg_values_supported": ["RS256"],
                "subject_types_supported": ["public"],
            }))
        elif path == "/jwks":
            self._send(200, json.dumps(jwks_dict()))
        elif path == "/authorize":
            q = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
            redirect_uri = q.get("redirect_uri", [""])[0]
            state = q.get("state", [""])[0]
            code_challenge = q.get("code_challenge", [""])[0]
            nonce = q.get("nonce", [""])[0]
            code = secrets.token_urlsafe(32)
            codes[code] = {
                "code_challenge": code_challenge,
                "nonce": nonce,
                "redirect_uri": redirect_uri,
            }
            sep = "&" if "?" in redirect_uri else "?"
            loc = f"{redirect_uri}{sep}code={code}&state={state}"
            self._send(302, "", headers={"Location": loc})
        else:
            self._send(404, json.dumps({"error": "not found"}))

    def do_POST(self):
        path = urllib.parse.urlparse(self.path).path
        if path == "/token":
            length = int(self.headers.get("Content-Length", 0))
            body = urllib.parse.parse_qs(self.rfile.read(length).decode())
            code = body.get("code", [""])[0]
            verifier = body.get("code_verifier", [""])[0]
            # openidconnect 用 Basic auth 发 client 凭证，不在 body；mock 固定一个客户端
            client_id = "test-client"
            rec = codes.pop(code, None)
            if not rec:
                self._send(400, json.dumps({"error": "invalid_grant"}))
                return
            # PKCE S256 校验
            expected = b64(hashlib.sha256(verifier.encode()).digest())
            print(f"[mock] body keys={list(body.keys())} verifier={verifier!r}", file=__import__('sys').stderr)
            print(f"[mock] challenge stored={rec['code_challenge']!r} computed={expected!r}", file=__import__('sys').stderr)
            if rec["code_challenge"] and expected != rec["code_challenge"]:
                self._send(400, json.dumps({"error": "invalid_grant", "description": "pkce mismatch"}))
                return
            id_token = sign_id_token("user-123", client_id, rec["nonce"])
            self._send(200, json.dumps({
                "access_token": secrets.token_urlsafe(32),
                "token_type": "Bearer",
                "expires_in": 3600,
                "id_token": id_token,
            }))
        else:
            self._send(404, json.dumps({"error": "not found"}))


if __name__ == "__main__":
    print(f"Mock OIDC IdP listening on {ISSUER}")
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()

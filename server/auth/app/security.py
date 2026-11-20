"""密码摘要和登录凭证。

* 密码：`PBKDF2-HMAC-SHA256`，每个账号独立随机盐（16 字节），迭代次数写在文件里，
  以后调大迭代次数老账号也还能登录（按各自记录的次数校验，升级时重算）。
* 登录凭证：32 字节随机串（base64url，43 个字符），服务端存一份，可以随时作废（退出登录）；
  不签名、不带内容，所以不需要密钥，泄露一个凭证也只影响那一次登录。
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import secrets

SALT_BYTES = 16
TOKEN_BYTES = 32


def _b64(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).decode().rstrip("=")


def new_salt() -> str:
    return _b64(secrets.token_bytes(SALT_BYTES))


def new_token() -> str:
    return _b64(secrets.token_bytes(TOKEN_BYTES))


def new_id() -> str:
    return "u" + _b64(secrets.token_bytes(9))


def hash_password(password: str, salt: str, rounds: int) -> str:
    """盐 + 密码 → 摘要（base64url）；同样的输入永远同样的输出"""
    raw = hashlib.pbkdf2_hmac("sha256", password.encode(), salt.encode(), rounds)
    return _b64(raw)


def verify_password(password: str, salt: str, expected: str, rounds: int) -> tuple[bool, bool]:
    """校验密码，返回 `(对不对, 要不要用新的迭代次数重算)`。

    第二个值是为了以后调大迭代次数：老账号按自己记录的次数校验，通过之后由调用方用当前次数重存。
    """
    ok = hmac.compare_digest(hash_password(password, salt, rounds), expected)
    return ok, ok


def token_expired(now: int, expires_at: int) -> bool:
    return now >= expires_at

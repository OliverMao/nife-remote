"""`NIFE_REMOTE_AUTH_RELAY_HOST` 原样透传，不拆分、不补端口。

客户端拿到 `GET /relay` 里的 `address` 后照着连（见 client/src-tauri/src/relay.rs）：写什么就是什么，
`auth.naig.us` 和 `192.11.11.11:4433` 都不会被改成带 `:4433` 的形式。

跑法（在 server/auth 目录，用 uv 建虚拟环境）：

    uv venv
    uv pip install -r requirements-dev.txt
    .venv/bin/python -m pytest -q            # Windows: .venv\\Scripts\\python -m pytest -q
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app import settings as settings_mod  # noqa: E402


@pytest.fixture(autouse=True)
def clean_env(monkeypatch):
    monkeypatch.delenv("NIFE_REMOTE_AUTH_RELAY_HOST", raising=False)


@pytest.mark.parametrize(
    "value",
    [
        "auth.naig.us",  # 只写域名：原样保留，不补 :4433
        "192.11.11.11:4433",  # 带端口：原样保留
        "relay.example.com:8443",
        "naig.us",  # 域名本身带点也照样透传
        "2001:db8::1",  # 纯 IPv6 原样
        "[2001:db8::1]:4433",  # 带方括号的 IPv6
    ],
)
def test_relay_address_is_passed_through_unchanged(value):
    import os

    os.environ["NIFE_REMOTE_AUTH_RELAY_HOST"] = value
    try:
        assert settings_mod.load().relay_address == value
    finally:
        os.environ.pop("NIFE_REMOTE_AUTH_RELAY_HOST", None)


def test_surrounding_whitespace_is_trimmed():
    import os

    os.environ["NIFE_REMOTE_AUTH_RELAY_HOST"] = "  auth.naig.us:4433  "
    try:
        assert settings_mod.load().relay_address == "auth.naig.us:4433"
    finally:
        os.environ.pop("NIFE_REMOTE_AUTH_RELAY_HOST")


def test_empty_when_not_configured():
    assert settings_mod.load().relay_address == ""

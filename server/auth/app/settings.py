"""账号服务的配置：全部来自环境变量，启动时读一次。

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `NIFE_REMOTE_AUTH_DATA` | `/data` | 数据目录（`accounts.json` 放在这里） |
| `NIFE_REMOTE_AUTH_DB` | `$NIFE_REMOTE_AUTH_DATA/accounts.json` | 账号文件，可以单独指定 |
| `NIFE_REMOTE_AUTH_TOKEN_TTL` | `2592000`（30 天） | 登录凭证的有效期（秒） |
| `NIFE_REMOTE_AUTH_MAX_USERS` | `1000` | 最多多少个账号 |
| `NIFE_REMOTE_AUTH_REGISTRATION` | `open` | `open` 允许注册，`closed` 关闭注册 |
| `NIFE_REMOTE_AUTH_ROUNDS` | `200000` | 密码摘要（PBKDF2-HMAC-SHA256）的迭代次数 |
| `NIFE_REMOTE_AUTH_DEFAULT_USER` | `niferemote` | 服务器上**一个账号都没有**时自动建的账号名（设成空就不建） |
| `NIFE_REMOTE_AUTH_DEFAULT_PASSWORD` | `niferemote` | 上面那个账号的初始密码（第一次登录后请改掉） |
| `NIFE_REMOTE_AUTH_RELAY_HOST` | 空 | 中转服务器的**完整地址**，原样交给客户端（`auth.naig.us`、`192.11.11.11:4433` 都行；服务不拆分、不补端口） |
| `NIFE_REMOTE_AUTH_RELAY_FP` | 空 | 中转服务器的证书指纹（空 = 客户端首次连接时信任，会记在客户端本地） |
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Settings:
    db_path: Path
    token_ttl: int
    max_users: int
    registration: str
    rounds: int
    default_user: str = "niferemote"
    default_password: str = "niferemote"
    relay_address: str = ""
    relay_fingerprint: str = ""

    @property
    def registration_open(self) -> bool:
        return self.registration != "closed"


def _int(name: str, default: int) -> int:
    raw = os.environ.get(name)
    if not raw:
        return default
    try:
        value = int(raw)
    except ValueError:
        return default
    return value if 0 < value <= 65535 else default


def load() -> Settings:
    data = Path(os.environ.get("NIFE_REMOTE_AUTH_DATA") or "/data")
    db = os.environ.get("NIFE_REMOTE_AUTH_DB")
    return Settings(
        db_path=Path(db) if db else data / "accounts.json",
        token_ttl=_int("NIFE_REMOTE_AUTH_TOKEN_TTL", 30 * 24 * 3600),
        max_users=_int("NIFE_REMOTE_AUTH_MAX_USERS", 1000),
        registration=(os.environ.get("NIFE_REMOTE_AUTH_REGISTRATION") or "open").strip().lower(),
        rounds=_int("NIFE_REMOTE_AUTH_ROUNDS", 200_000),
        # 默认账号：没设这个环境变量就建 niferemote；显式设成空（NIFE_REMOTE_AUTH_DEFAULT_USER=）才不建
        default_user=(
            "niferemote"
            if os.environ.get("NIFE_REMOTE_AUTH_DEFAULT_USER") is None
            else (os.environ.get("NIFE_REMOTE_AUTH_DEFAULT_USER") or "").strip()
        ),
        default_password=os.environ.get("NIFE_REMOTE_AUTH_DEFAULT_PASSWORD") or "niferemote",
        relay_address=(os.environ.get("NIFE_REMOTE_AUTH_RELAY_HOST") or "").strip(),
        relay_fingerprint=(os.environ.get("NIFE_REMOTE_AUTH_RELAY_FP") or "").strip(),
    )

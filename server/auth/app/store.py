"""账号存储：一个 JSON 文件（`accounts.json`），进程内一把 `asyncio.Lock` 串行化写入。

结构：

```json
{
  "version": 1,
  "users": [
    {
      "id": "u1a2b3c4",
      "name": "mao",
      "name_lower": "mao",
      "salt": "…",
      "hash": "…",
      "rounds": 200000,
      "createdAt": 1731000000,
      "devices": ["260778"]
    }
  ],
  "tokens": [{ "token": "…", "userId": "u1a2b3c4", "createdAt": 1731000000, "expiresAt": 1733000000 }]
}
```

为什么是 JSON 而不是数据库：账号量很小（一台服务器几十到几千个），一次读写整个文件足够快，
备份就是 `cp accounts.json`。写文件是原子的（写 `accounts.json.tmp` → `os.replace`），进程被杀不会留下半个文件。
同一时刻只有一个进程能写：多 worker 部署时要挂同一块卷并设 `NIFE_REMOTE_AUTH_WORKERS=1`（见 README）。
"""

from __future__ import annotations

import json
import os
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import security

VERSION = 1


class StoreError(Exception):
    """账号层面的错误（重名、密码不对、凭证无效…），`str(e)` 直接给界面显示"""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


@dataclass
class User:
    id: str
    name: str
    salt: str
    hash: str
    rounds: int
    created_at: int
    devices: list[str] = field(default_factory=list)

    def to_json(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "name_lower": self.name.lower(),
            "salt": self.salt,
            "hash": self.hash,
            "rounds": self.rounds,
            "createdAt": self.created_at,
            "devices": list(self.devices),
        }

    def public(self) -> dict[str, Any]:
        """给界面看的（不含盐和摘要）"""
        return {"id": self.id, "name": self.name, "createdAt": self.created_at, "devices": list(self.devices)}


@dataclass
class Token:
    token: str
    user_id: str
    created_at: int
    expires_at: int

    def to_json(self) -> dict[str, Any]:
        return {
            "token": self.token,
            "userId": self.user_id,
            "createdAt": self.created_at,
            "expiresAt": self.expires_at,
        }


class AccountStore:
    """一个 JSON 文件里的账号和登录凭证。线程安全（后台线程写文件时也用同一把锁）。"""

    def __init__(self, path: Path, *, rounds: int, token_ttl: int, max_users: int) -> None:
        self.path = path
        self.rounds = rounds
        self.token_ttl = token_ttl
        self.max_users = max_users
        self._lock = threading.RLock()
        self._users: list[User] = []
        self._tokens: list[Token] = []
        self.load()

    # ── 读写文件 ──

    def load(self) -> None:
        with self._lock:
            if not self.path.exists():
                self._users, self._tokens = [], []
                return
            try:
                raw = json.loads(self.path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                # 读坏了就当没有账号：不能因为一个坏文件让服务起不来
                self._users, self._tokens = [], []
                return
            self._users = [
                User(
                    id=u["id"],
                    name=u["name"],
                    salt=u["salt"],
                    hash=u["hash"],
                    rounds=int(u.get("rounds") or self.rounds),
                    created_at=int(u.get("createdAt") or 0),
                    devices=[str(d) for d in u.get("devices") or []],
                )
                for u in raw.get("users") or []
            ]
            self._tokens = [
                Token(
                    token=t["token"],
                    user_id=t["userId"],
                    created_at=int(t.get("createdAt") or 0),
                    expires_at=int(t.get("expiresAt") or 0),
                )
                for t in raw.get("tokens") or []
            ]

    def _write(self) -> None:
        """原子写：临时文件 + `os.replace`"""
        data = {
            "version": VERSION,
            "users": [u.to_json() for u in self._users],
            "tokens": [t.to_json() for t in self._tokens],
        }
        self.path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_suffix(".json.tmp")
        with tmp.open("w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
            f.write("\n")
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, self.path)
        try:  # 文件里有盐和摘要，别人别读
            self.path.chmod(0o600)
        except OSError:
            pass

    # ── 查询 ──

    def _find(self, name: str) -> User | None:
        key = name.strip().lower()
        return next((u for u in self._users if u.name.lower() == key), None)

    def _find_token(self, token: str) -> Token | None:
        now = int(time.time())
        # 顺手清掉过期的
        self._tokens = [t for t in self._tokens if t.expires_at > now]
        return next((t for t in self._tokens if t.token == token), None)

    @property
    def user_count(self) -> int:
        with self._lock:
            return len(self._users)

    # ── 注册 / 登录 / 退出 ──

    def register(self, name: str, password: str) -> tuple[User, Token]:
        name = check_name(name)
        check_password(password)
        with self._lock:
            if self._find(name) is not None:
                raise StoreError("name_taken", "that account name is already taken")
            if len(self._users) >= self.max_users:
                raise StoreError("too_many_users", f"this server allows at most {self.max_users} accounts")
            salt = security.new_salt()
            user = User(
                id=security.new_id(),
                name=name,
                salt=salt,
                hash=security.hash_password(password, salt, self.rounds),
                rounds=self.rounds,
                created_at=int(time.time()),
            )
            self._users.append(user)
            token = self._issue(user)
            self._write()
            return user, token

    def login(self, name: str, password: str) -> tuple[User, Token]:
        with self._lock:
            user = self._find(name)
            if user is None:
                raise StoreError("no_such_user", "no such account on this server")
            ok, _ = security.verify_password(password, user.salt, user.hash, user.rounds)
            if not ok:
                raise StoreError("wrong_password", "wrong password")
            # 迭代次数调大过：登录成功顺手升级这个账号的摘要
            if user.rounds != self.rounds:
                user.hash = security.hash_password(password, user.salt, self.rounds)
                user.rounds = self.rounds
            token = self._issue(user)
            self._write()
            return user, token

    def logout(self, token: str) -> bool:
        with self._lock:
            before = len(self._tokens)
            self._tokens = [t for t in self._tokens if t.token != token]
            removed = len(self._tokens) != before
            if removed:
                self._write()
            return removed

    def _issue(self, user: User) -> Token:
        now = int(time.time())
        token = Token(
            token=security.new_token(),
            user_id=user.id,
            created_at=now,
            expires_at=now + self.token_ttl,
        )
        self._tokens = [t for t in self._tokens if t.user_id != user.id or t.expires_at > now]
        self._tokens.append(token)
        return token

    def change_password(self, user: User, old_password: str, new_password: str) -> None:
        """改密码：要验旧的；改完这个账号之前发出的凭证全部作废（别的设备要重新登录）"""
        check_password(new_password)
        with self._lock:
            target = next((u for u in self._users if u.id == user.id), None)
            if target is None:
                raise StoreError("bad_token", "this login is no longer valid, sign in again")
            ok, _ = security.verify_password(old_password, target.salt, target.hash, target.rounds)
            if not ok:
                raise StoreError("wrong_password", "the current password is wrong")
            # 换盐重算（同一个密码也不会得到同样的摘要）
            target.salt = security.new_salt()
            target.rounds = self.rounds
            target.hash = security.hash_password(new_password, target.salt, self.rounds)
            self._tokens = [t for t in self._tokens if t.user_id != user.id]
            self._write()

    # ── 凭证 ──

    def user_for_token(self, token: str) -> User:
        with self._lock:
            found = self._find_token(token)
            if found is None:
                raise StoreError("bad_token", "this login is no longer valid, sign in again")
            user = next((u for u in self._users if u.id == found.user_id), None)
            if user is None:
                raise StoreError("bad_token", "this login is no longer valid, sign in again")
            return user

    # ── 账号下的设备（"本账号设备"）──

    def set_devices(self, user: User, devices: list[str]) -> User:
        with self._lock:
            target = next((u for u in self._users if u.id == user.id), None)
            if target is None:
                raise StoreError("bad_token", "this login is no longer valid, sign in again")
            target.devices = normalize_codes(devices)
            self._write()
            return target


# ── 校验 ──

NAME_MIN, NAME_MAX = 2, 20
PASSWORD_MIN, PASSWORD_MAX = 6, 128


def check_name(name: str) -> str:
    name = (name or "").strip()
    if not (NAME_MIN <= len(name) <= NAME_MAX):
        raise StoreError("bad_name", f"account name must be {NAME_MIN} to {NAME_MAX} characters")
    if not all(c.isalnum() or c in "_-." for c in name):
        raise StoreError("bad_name", "account name can only contain letters, digits, _ - .")
    return name


def check_password(password: str) -> None:
    length = len(password or "")
    if not (PASSWORD_MIN <= length <= PASSWORD_MAX):
        raise StoreError("bad_password", f"password must be {PASSWORD_MIN} to {PASSWORD_MAX} characters")


def normalize_codes(codes: list[str]) -> list[str]:
    """只留 6 位数字且不以 0 开头的设备ID，去重保序"""
    out: list[str] = []
    for code in codes:
        digits = "".join(c for c in str(code) if c.isdigit())
        if len(digits) == 6 and digits[0] != "0" and digits not in out:
            out.append(digits)
    return out

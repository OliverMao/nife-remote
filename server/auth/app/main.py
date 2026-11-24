"""HTTP 接口（给桌面客户端和以后的网页用）。

```
POST /auth/register   {name, password}          → {token, user}
POST /auth/login      {name, password}          → {token, user}
POST /auth/logout     Bearer <token>            → {ok: true}
GET  /auth/me         Bearer <token>            → {user}
GET  /auth/devices    Bearer <token>            → {devices: [{code, key, name, os, online}]}
POST /auth/heartbeat  Bearer <token> {code}     → {devices: [...]}
WS   /auth/ws?token=<token>                     ← 设备报到（见下）
PUT  /auth/devices    Bearer <token> {devices}  → {devices: [...]}
GET  /relay                                     → {relay: {address, fingerprint}}
GET  /health                                    → {ok: true, users: n, version}
```

* 错误一律是 `{"error": {"code": "...", "message": "..."}}`，HTTP 状态码同时对应
  （`400` 参数不对 / 重名 / 密码不对，`401` 凭证失效，`403` 关闭注册，`404` 没这个账号，`429` 太多次登录失败）。
* 登录失败做过频率限制：同一个来源 IP 一分钟内最多 10 次失败，超过返回 `429`。
* 账号文件是 `accounts.json`（见 [`store`](store.py)），`NIFE_REMOTE_AUTH_DB` 可以换位置。

# 设备连接（`/auth/ws`）

每台设备启动后开一条 WebSocket 连到这里（`?token=<登录凭证>`），第一条消息报到：

```json
{"type": "hello", "code": "258374", "name": "MAO7", "os": "Windows 11 Home China 25H2"}
```

服务把它记进账号的设备列表（控制码属于哪个账号也在这一步定下来，见 `store.claim_device`），
然后给这个账号的**所有**设备连接推一条完整列表：

```json
{"type": "devices", "devices": [{"code": "258374", "name": "MAO7", "os": "...", "key": "…", "online": true}]}
```

之后有设备上下线、改名、换控制码都会再推一次。设备可以发 `{"type": "ping"}` 保活（服务回 `{"type": "pong"}`）。
报到的控制码属于别的账号时回 `{"type": "error", "code": "device_taken", ...}`，那条连接仍然留着（可以再报别的码）。
"""

from __future__ import annotations

import time
import logging
from contextlib import asynccontextmanager
from typing import Any

from fastapi import Depends, FastAPI, Header, Query, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field

from . import settings as settings_mod
from .presence import HUB
from .store import AccountStore, StoreError, User

VERSION = "0.0.1"
logger = logging.getLogger("nife-remote-auth")
LOGIN_FAIL_WINDOW = 60
LOGIN_FAIL_LIMIT = 10

settings = settings_mod.load()
store = AccountStore(
    settings.db_path,
    rounds=settings.rounds,
    token_ttl=settings.token_ttl,
    max_users=settings.max_users,
)

def seed_default_account() -> None:
    """服务器上一个账号都没有时建一个默认账号（`niferemote`/`niferemote`），方便第一次登录。

    建完之后请马上用 `POST /auth/password` 改密码；`NIFE_REMOTE_AUTH_DEFAULT_USER=` （设成空）可以关掉这个行为。
    """
    if store.user_count > 0:
        return
    if not settings.default_user:
        return
    try:
        user, _token = store.register(settings.default_user, settings.default_password)
        logger.warning(
            "created the default account %r with the password from NIFE_REMOTE_AUTH_DEFAULT_PASSWORD; "
            "sign in and change it right away",
            user.name,
        )
    except StoreError as e:
        logger.warning("can't create the default account: %s", e.message)


@asynccontextmanager
async def lifespan(_app: FastAPI):
    seed_default_account()
    yield


app = FastAPI(
    title="Nife Remote accounts",
    version=VERSION,
    description="账号和账号下的设备列表。和中转服务器（QUIC，见 server/core）互不影响，可以单独部署。",
    lifespan=lifespan,
)

# 来源 IP → (窗口开始的时刻, 失败次数)
_failures: dict[str, tuple[float, int]] = {}


class Credentials(BaseModel):
    name: str = Field(..., description="账号名，2–20 个字符")
    password: str = Field(..., description="密码，6–128 个字符")


class PasswordChange(BaseModel):
    old_password: str = Field(..., description="现在的密码")
    new_password: str = Field(..., description="新密码，6–128 个字符")


class DeviceEntry(BaseModel):
    code: str = Field(..., description="设备ID（6 位数字）")
    # 这台设备的控制台端到端公钥（64 位十六进制）：账号里其它设备用它认出它，不用输配对码
    key: str = Field("", description="控制台公钥")


class Heartbeat(BaseModel):
    code: str = Field(..., description="这台设备的控制码（6 位数字）")


class DeviceList(BaseModel):
    devices: list[DeviceEntry] = Field(default_factory=list, description="登录这个账号的设备")


def error(status: int, code: str, message: str) -> JSONResponse:
    return JSONResponse(status_code=status, content={"error": {"code": code, "message": message}})


def _fail_key(request: Request) -> str:
    return request.client.host if request.client else "?"


def _too_many(request: Request) -> bool:
    key = _fail_key(request)
    now = time.time()
    start, count = _failures.get(key, (now, 0))
    if now - start > LOGIN_FAIL_WINDOW:
        _failures.pop(key, None)
        return False
    return count >= LOGIN_FAIL_LIMIT


def _note_failure(request: Request) -> None:
    key = _fail_key(request)
    now = time.time()
    start, count = _failures.get(key, (now, 0))
    if now - start > LOGIN_FAIL_WINDOW:
        _failures[key] = (now, 1)
    else:
        _failures[key] = (start, count + 1)


def _bearer(authorization: str | None) -> str:
    if not authorization or not authorization.lower().startswith("bearer "):
        raise StoreError("bad_token", "missing login token")
    return authorization[7:].strip()


def current_user(authorization: str | None = Header(default=None)) -> User:
    return store.user_for_token(_bearer(authorization))


def _session(user: User, token: str) -> dict[str, Any]:
    return {"token": token, "user": user.public()}


@app.exception_handler(StoreError)
async def store_error_handler(request: Request, exc: StoreError) -> JSONResponse:
    """账号层面的错误 → 统一的 `{"error": {...}}` + 对应的状态码"""
    status = {
        "bad_token": 401,
        "wrong_password": 401,
        "no_such_user": 404,
        "name_taken": 400,
        "bad_name": 400,
        "bad_password": 400,
        "too_many_users": 400,
        "closed": 403,
        "too_many_attempts": 429,
    }.get(exc.code, 400)
    return error(status, exc.code, exc.message)


@app.get("/relay")
def relay() -> dict[str, Any]:
    """中转服务器在哪：客户端只需要填账号服务地址，中转服务器的地址和指纹由这里分配。

    `address` 就是 `NIFE_REMOTE_AUTH_RELAY_HOST` 的原样内容（`auth.naig.us`、
    `192.11.11.11:4433` 都行），服务不拆分、不补端口，客户端照着连。

    以后要做"不同账号走不同路径 / 多线路 / 灰度"，改这里和 `settings` 就够了，客户端不用改。
    """
    return {
        "relay": {
            "address": settings.relay_address,
            "fingerprint": settings.relay_fingerprint,
        }
    }


@app.get("/health")
def health() -> dict[str, Any]:
    return {
        "ok": True,
        "version": VERSION,
        "users": store.user_count,
        "registration": settings.registration,
        "relay": settings.relay_address or None,
    }


@app.post("/auth/register")
def register(body: Credentials, request: Request):
    if not settings.registration_open:
        raise StoreError("closed", "registration is closed on this server")
    if _too_many(request):
        raise StoreError("too_many_attempts", "too many attempts, try again in a minute")
    try:
        user, token = store.register(body.name, body.password)
    except StoreError as e:
        _note_failure(request)
        raise e
    return _session(user, token.token)


@app.post("/auth/login")
def login(body: Credentials, request: Request):
    if _too_many(request):
        raise StoreError("too_many_attempts", "too many attempts, try again in a minute")
    try:
        user, token = store.login(body.name, body.password)
    except StoreError as e:
        _note_failure(request)
        raise e
    return _session(user, token.token)


@app.post("/auth/logout")
def logout(authorization: str | None = Header(default=None)):
    token = _bearer(authorization)
    store.user_for_token(token)  # 凭证不对就不要假装成功
    store.logout(token)
    return {"ok": True}


@app.get("/auth/me")
def me(user: User = Depends(current_user)):
    return {"user": user.public()}


@app.post("/auth/password")
def change_password(body: PasswordChange, user: User = Depends(current_user)):
    store.change_password(user, body.old_password, body.new_password)
    # 改完所有凭证都作废了，这里重新发一个给当前设备
    _fresh, token = store.login(user.name, body.new_password)
    return {"ok": True, "token": token.token}


def devices_payload(user: User) -> dict[str, Any]:
    """账号下的设备列表（HTTP 和 WebSocket 用同一份）：在线状态来自设备连接（见 presence）"""
    online = HUB.online(user.id)
    devices = []
    for entry in user.device_entries():
        devices.append({**entry, "online": entry["online"] or entry["code"] in online})
    return {"type": "devices", "devices": devices}


@app.websocket("/auth/ws")
async def device_socket(ws: WebSocket, token: str = Query("", description="登录凭证")):
    """设备的常连接：报到、上下线推送（见模块说明的"设备连接"）"""
    try:
        user = store.user_for_token(token)
    except StoreError as e:
        await ws.close(code=4401, reason=e.code)
        return
    await ws.accept()
    conn = await HUB.join(user.id, ws)
    try:
        await ws.send_json(devices_payload(user))
        # 别的设备早就上线的，第一条推送里就有；这台设备自己报到的码在 hello 之后再加上
        while True:
            raw = await ws.receive_json()
            kind = raw.get("type") if isinstance(raw, dict) else None
            if kind == "hello":
                code = str(raw.get("code") or "")
                name = str(raw.get("name") or "")
                os_name = str(raw.get("os") or "")
                try:
                    user = store.claim_device(user, code, name, os_name)
                except StoreError as e:
                    await ws.send_json({"type": "error", "code": e.code, "message": e.message})
                    continue
                conn.codes.add(code.zfill(6))
                await HUB.broadcast(user.id, devices_payload(user))
            elif kind == "ping":
                await ws.send_json({"type": "pong"})
            elif kind == "bye":
                conn.codes.clear()
                await HUB.broadcast(user.id, devices_payload(user))
    except (WebSocketDisconnect, RuntimeError, ValueError):
        pass
    finally:
        await HUB.leave(conn)
        await HUB.broadcast(user.id, devices_payload(user))


@app.get("/auth/devices")
def get_devices(user: User = Depends(current_user)):
    return devices_payload(user)


@app.post("/auth/heartbeat")
def heartbeat(body: Heartbeat, user: User = Depends(current_user)):
    """设备报到（每 30 秒一次）：其它设备据此看到它在线（见 `DeviceEntry.online`）。"""
    store.heartbeat(user, body.code)
    return devices_payload(user)


@app.put("/auth/devices")
def put_devices(body: DeviceList, user: User = Depends(current_user)):
    updated = store.set_devices(user, [d.model_dump() for d in body.devices])
    return devices_payload(updated)

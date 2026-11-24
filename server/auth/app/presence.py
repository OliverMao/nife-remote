"""设备的在线状态：每台设备用一条 WebSocket 连到账号服务上。

为什么是 WebSocket 而不是 HTTP 心跳：

* 设备一上线 / 下线，同一个账号的其它设备**马上**看到（HTTP 心跳最多慢一个轮询周期）；
* 控制码属于哪个账号也在这条连接上确认（见 `store.claim_device`），一台设备不需要再各自登记；
* 列表、名字、系统、在线状态都由服务端推给所有相关设备，客户端不用自己算。

一条连接通常只代表一台设备（`hello` 里带控制码），但服务不限：同一个账号开多个客户端时，各自报各自的码。
在线 = 这个账号下**至少有一条连接**报了这个码。
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from typing import Any


@dataclass(eq=False)
class Conn:
    """一条设备连接"""

    user_id: str
    ws: Any
    # 这条连接报上来的控制码（一般就一个）
    codes: set[str] = field(default_factory=set)

    async def send(self, payload: dict[str, Any]) -> None:
        try:
            await self.ws.send_json(payload)
        except Exception:  # 对端已经走了：leave 会把它摘掉
            pass


class Hub:
    """账号 → 在线的设备连接"""

    def __init__(self) -> None:
        self._conns: dict[str, set[Conn]] = {}
        self._lock = asyncio.Lock()

    async def join(self, user_id: str, ws: Any) -> Conn:
        conn = Conn(user_id=user_id, ws=ws)
        async with self._lock:
            self._conns.setdefault(user_id, set()).add(conn)
        return conn

    async def leave(self, conn: Conn) -> None:
        async with self._lock:
            conns = self._conns.get(conn.user_id)
            if conns is not None:
                conns.discard(conn)
                if not conns:
                    self._conns.pop(conn.user_id, None)

    def online(self, user_id: str) -> set[str]:
        """这个账号现在在线的控制码"""
        return {code for conn in self._conns.get(user_id, ()) for code in conn.codes}

    async def broadcast(self, user_id: str, payload: dict[str, Any]) -> None:
        """推给这个账号的所有设备连接"""
        conns = list(self._conns.get(user_id, ()))
        if not conns:
            return
        await asyncio.gather(*(c.send(payload) for c in conns))


# 进程内一个（见 app.main 的 /auth/ws）
HUB = Hub()

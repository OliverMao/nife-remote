"""设备的在线状态 + 同账号设备之间的文件传输转发（每台设备一条 WebSocket 连到账号服务上）。

为什么是 WebSocket 而不是 HTTP 心跳：

* 设备一上线 / 下线，同一个账号的其它设备**马上**看到（HTTP 心跳最多慢一个轮询周期）；
* 控制码属于哪个账号也在这条连接上确认（见 `store.claim_device`），一台设备不需要再各自登记；
* 列表、名字、系统、在线状态都由服务端推给所有相关设备，客户端不用自己算。

一条连接通常只代表一台设备（`hello` 里带控制码），但服务不限：同一个账号开多个客户端时，各自报各自的码。
在线 = 这个账号下**至少有一条连接**报了这个码。

文件传输也走这条连接（见下面的"文件传输转发"）：两台设备都在线时，一条隧道就是"把 A 发出的
`data` 原样交给 B、把 B 的回答原样交给 A"。服务不解析内容，只认隧道号——所以协议、块大小
以后怎么改都不用动这里。
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


@dataclass(eq=False)
class Tunnel:
    """两台同账号设备之间的一条文件传输隧道（服务只转发，不看内容）"""

    a: Conn
    b: Conn
    code_a: str
    code_b: str

    def peer_of(self, conn: Conn) -> Conn:
        return self.b if conn is self.a else self.a


class Hub:
    """账号 → 在线的设备连接；隧道号 → 隧道"""

    def __init__(self) -> None:
        self._conns: dict[str, set[Conn]] = {}
        self._tunnels: dict[int, Tunnel] = {}
        self._next = 1
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
            # 这条连接上的隧道全部拆掉，让对面别再等（带着各自的隧道号）
            gone = [
                (tid, t.peer_of(conn))
                for tid, t in self._tunnels.items()
                if conn in (t.a, t.b)
            ]
            for tid, _peer in gone:
                self._tunnels.pop(tid, None)
        for tid, peer in gone:
            await peer.send({"type": "fs-gone", "id": tid, "reason": "peerGone"})

    def online(self, user_id: str) -> set[str]:
        """这个账号现在在线的控制码"""
        return {code for conn in self._conns.get(user_id, ()) for code in conn.codes}

    async def broadcast(self, user_id: str, payload: dict[str, Any]) -> None:
        """推给这个账号的所有设备连接"""
        conns = list(self._conns.get(user_id, ()))
        if not conns:
            return
        await asyncio.gather(*(c.send(payload) for c in conns))

    # ─────────────── 文件传输转发 ───────────────

    def _conn_for(self, user_id: str, code: str) -> Conn | None:
        """这个账号里现在报着 `code` 的连接"""
        for conn in self._conns.get(user_id, ()):
            if code in conn.codes:
                return conn
        return None

    async def open_tunnel(self, conn: Conn, to: str, use: int) -> None:
        """`conn` 要和同账号的 `to` 传文件：两边都知道隧道号，之后只转 `data`

        `to` 不在这个账号里 / 不在线 → 回 `fs-error`（`use` 原样带回，调用方据此对号）
        """
        code_a = next(iter(conn.codes), "")
        peer = self._conn_for(conn.user_id, to)
        if peer is None or peer is conn:
            await conn.send({"type": "fs-error", "use": use, "code": "peerOffline"})
            return
        async with self._lock:
            tid = self._next
            self._next += 1
            self._tunnels[tid] = Tunnel(a=conn, b=peer, code_a=code_a, code_b=to)
        await conn.send({"type": "fs-opened", "use": use, "id": tid})
        await peer.send({"type": "fs-call", "id": tid, "from": code_a})

    async def relay(self, conn: Conn, tid: int, data: Any) -> None:
        """把一方的 `data` 原样交给另一方"""
        tunnel = self._tunnels.get(tid)
        if tunnel is None or conn not in (tunnel.a, tunnel.b):
            # 隧道已经被拆了（对面掉线）：告诉发送方，别一直等
            await conn.send({"type": "fs-gone", "id": tid, "reason": "closed"})
            return
        await tunnel.peer_of(conn).send({"type": "fs-data", "id": tid, "data": data})

    async def close_tunnel(self, conn: Conn, tid: int) -> None:
        async with self._lock:
            tunnel = self._tunnels.pop(tid, None)
        if tunnel is None:
            return
        await tunnel.peer_of(conn).send({"type": "fs-gone", "id": tid, "reason": "closed"})


# 进程内一个（见 app.main 的 /auth/ws）
HUB = Hub()

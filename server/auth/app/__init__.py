"""Nife Remote 账号服务（FastAPI）。

和中转服务器（`server/core`，QUIC）是两个独立的服务：这个只做账号和"本账号设备"，
不参与任何画面 / 键鼠的中转，部署挂了也不影响远程控制本身。
"""

__all__ = ["main", "settings", "store", "security"]

# 账号服务（`server/auth`）

FastAPI + 一个 JSON 文件（`accounts.json`）的账号服务：注册、登录、退出、账号信息、"本账号设备"列表。
**它不参与任何画面 / 键鼠中转**——那部分是 `server/core`（Rust + QUIC）。两个服务独立部署，账号服务挂了不影响远程控制本身。

```
server/
├── core/        # 中转服务器（Rust + QUIC），原来就在这里的代码，只是挪进了 core/
│   ├── Cargo.toml
│   ├── Dockerfile
│   └── src/
└── auth/        # 账号服务（Python + FastAPI）
    ├── app/
    │   ├── main.py       # 路由
    │   ├── store.py      # accounts.json 的读写（原子写）
    │   ├── security.py   # PBKDF2 密码摘要、登录凭证
    │   └── settings.py   # 环境变量
    ├── tests/
    └── Dockerfile
```

## 接口

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/relay` | `{relay: {host, port, fingerprint}}`：**中转服务器由这里分配**，客户端只填账号服务地址 |
| `GET` | `/health` | `{ok, version, users, registration, relay}` |
| `POST` | `/auth/register` | `{name, password}` → `{token, user}` |
| `POST` | `/auth/login` | `{name, password}` → `{token, user}` |
| `POST` | `/auth/logout` | `Bearer <token>` → `{ok:true}` |
| `GET` | `/auth/me` | `Bearer <token>` → `{user}` |
| `GET` | `/auth/devices` | `Bearer <token>` → `{devices:["260778"]}` |
| `PUT` | `/auth/devices` | `Bearer <token>` `{devices}` → `{devices}` |
| `POST` | `/auth/password` | `Bearer <token>` `{oldPassword, newPassword}` → `{ok, token}` |

错误统一是 `{"error": {"code": "...", "message": "..."}}`：

| 状态码 | code | 什么时候 |
| --- | --- | --- |
| 400 | `bad_name` / `bad_password` / `name_taken` / `too_many_users` | 注册参数不对、重名 |
| 401 | `wrong_password` / `bad_token` | 密码不对、凭证过期或被作废 |
| 403 | `closed` | `NIFE_REMOTE_AUTH_REGISTRATION=closed` 时注册 |
| 404 | `no_such_user` | 登录一个不存在的账号 |
| 429 | `too_many_attempts` | 同一 IP 一分钟内失败 10 次 |

## 存储

`accounts.json`（`NIFE_REMOTE_AUTH_DB`，默认 `/data/accounts.json`）：

```json
{
  "version": 1,
  "users": [{ "id": "u…", "name": "mao", "nameLower": "mao", "salt": "…", "hash": "…", "rounds": 200000, "createdAt": 1731000000, "devices": ["260778"] }],
  "tokens": [{ "token": "…", "userId": "u…", "createdAt": 1731000000, "expiresAt": 1733593000 }]
}
```

- 密码用 PBKDF2-HMAC-SHA256 + 每账号 16 字节随机盐，只存摘要；迭代次数写在账号里，以后调大时老账号登录成功会自动升级
- 登录凭证是 32 字节随机串，服务端存一份，可以随时作废（退出登录）；过期时间默认 30 天
- 写文件是原子的（`.tmp` → `os.replace`），权限 `0600`；读坏了当"没有账号"，服务照常起来
- 账号量小（默认上限 1000），一次读写整个文件足够快；备份就是 `cp accounts.json`
- **只能跑 1 个 worker**（`--workers 1`）：多进程会互相覆盖文件

## 本地跑

```bash
cd server/auth
python -m pip install -r requirements-dev.txt
NIFE_REMOTE_AUTH_DATA=./data uvicorn app.main:app --port 8080 --reload
curl -s localhost:8080/health
```

## 容器

账号服务是**仓库根 `docker-compose.yml` 里的一个 service**，和中转服务器共用同一份 `.env`、同一个 `docker compose up -d`：

```bash
# 部署机（仓库根目录）
cp .env.example .env      # NIFE_REMOTE_IMAGE 和 NIFE_REMOTE_AUTH_IMAGE 都在这里
docker compose pull && docker compose up -d
curl -s localhost:8080/health

# 只跑账号服务
docker compose up -d nife-remote-auth

# 不想用账号服务：把 compose 里的 nife-remote-auth 整段注释掉（或只 up 中转服务器）即可

# 构建镜像（构建机，仓库根目录）
docker build --network=host -f server/auth/Dockerfile -t <registry>/nife-remote-auth:0.0.1 server/auth
docker push <registry>/nife-remote-auth:0.0.1
```

数据在仓库根目录的 `data/auth/accounts.json`（挂到容器的 `/data`）。

对外建议放在 HTTPS 反向代理后面（Caddy：`handle /auth/* { reverse_proxy 127.0.0.1:8080 }`、`handle /health { reverse_proxy 127.0.0.1:8080 }`）。

## 客户端怎么用

桌面端的账号命令（`client/src-tauri/src/accounts.rs`）就是调这些接口，地址依次取：

1. 环境变量 `NIFE_REMOTE_AUTH`（例如 `https://relay.example.com`）
2. 构建时编进去的默认值（`option_env!("NIFE_REMOTE_AUTH")`）
3. `http://127.0.0.1:8080`（本机调试）

登录凭证存在本机 `session.json`（和 `config.toml` 同目录，权限 600），密码从不上传后再落盘。

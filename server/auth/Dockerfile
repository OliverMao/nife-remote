# syntax=docker/dockerfile:1
# ── Nife Remote 账号服务（FastAPI）────────────────────────────
# 和中转服务器（server/core）完全独立：不参与画面 / 键鼠中转，挂了也不影响远程控制。
#
#   构建（仓库根目录）：docker build -f server/auth/Dockerfile -t <registry>/nife-remote-auth:0.0.1 server/auth
#   运行：docker compose -f server/auth/docker-compose.yml up -d
FROM python:3.12-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    NIFE_REMOTE_AUTH_DATA=/data

WORKDIR /app
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

COPY app ./app

# 数据目录挂进来（accounts.json）：以 root 运行，挂载卷不管属于谁都写得进去
ENV NIFE_REMOTE_AUTH_DB=/data/accounts.json
VOLUME ["/data"]
EXPOSE 8080

# 单个 uvicorn worker：账号文件是"读整个文件 + 原子写回"，多 worker 会互相覆盖
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8080", "--workers", "1", "--proxy-headers"]

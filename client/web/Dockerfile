# syntax=docker/dockerfile:1
# Nife Remote 网页控制台：控制端跑在容器里，在浏览器里使用。构建上下文是仓库根目录（见 client/web/docker-compose.yml）。
# ── 控制台界面（React，与桌面版同一份）──────────────────
FROM node:22-bookworm-slim AS ui
WORKDIR /ui
COPY client/package.json client/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY client/index.html client/vite.config.ts client/tsconfig.json ./
COPY client/public ./public
COPY client/src ./src
RUN npm run build

# ── 服务（Rust）──────────────────────────────────────
# client/web 是独立的 Cargo 工作区，只用到 crates/ 里的两个库，不需要根目录的工作区和中转服务器
FROM rust:1-slim-bookworm AS builder
# 目录结构和仓库里一样（/src/client/web）：Cargo.toml 里的 ../../crates/... 不用改
WORKDIR /src/client/web
COPY crates /src/crates
COPY client/web/Cargo.toml client/web/Cargo.lock ./
COPY client/web/src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/client/web/target \
    cargo build --release --locked \
 && cp target/release/nife-remote-web /nife-remote-web

# ── 运行（distroless，无 shell，以 root 运行）────────────
# 以 root 运行（不用 :nonroot 标签、不设 USER）：挂载进来的 /data 不管属于谁都写得进去。目录不存在时 Docker 替你建的属于 root，
# 自己 mkdir 的属于你的用户，nonroot（65532）都写不进去，服务会因为写不了证书而退出。代价：data/ 里生成的文件在宿主机上属于 root。
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /nife-remote-web /usr/local/bin/nife-remote-web
COPY --from=ui /ui/dist /usr/share/nife-remote/www
ENV NIFE_REMOTE_WEB_LISTEN=0.0.0.0:8080 \
    NIFE_REMOTE_WEB_ROOT=/usr/share/nife-remote/www \
    NIFE_REMOTE_WEB_CERT=/data/cert.pem \
    NIFE_REMOTE_WEB_KEY=/data/key.pem \
    NIFE_REMOTE_SERVER_TRUST=/data/server-trust.txt \
    NIFE_REMOTE_WEB_IDENTITY=/data/controller-key.txt \
    RUST_LOG=info
VOLUME ["/data"]
# HTTP 和 HTTPS 共用这一个端口
EXPOSE 8080/tcp
ENTRYPOINT ["/usr/local/bin/nife-remote-web"]

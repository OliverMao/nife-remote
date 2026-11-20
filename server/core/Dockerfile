# syntax=docker/dockerfile:1
# ── 构建 ──────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY server/core ./server/core
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p nife-remote-server \
 && cp target/release/nife-remote-server /nife-remote-server

# ── 运行（distroless，无 shell，以 root 运行）────────────
# 以 root 运行（不用 :nonroot 标签、不设 USER、不建普通用户）：挂载进来的 /data 不管属于谁都写得进去。
# 目录不存在时 Docker 替你建的属于 root，自己 mkdir 的属于你的用户，换个非 root 用户就都写不进去，
# 服务会因为写不了证书而退出。代价：data/ 里生成的文件在宿主机上属于 root（读取、备份、删除要用 sudo）。
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /nife-remote-server /usr/local/bin/nife-remote-server
ENV NIFE_REMOTE_LISTEN=0.0.0.0:4433 \
    NIFE_REMOTE_CERT=/data/cert.pem \
    NIFE_REMOTE_KEY=/data/key.pem \
    RUST_LOG=info
# 明确指定 root（uid 0/gid 0）：基础镜像将来默认用户变了也不会被悄悄改成非 root，
# 也不要在 compose 或这里加 user: / USER <非 0> / adduser
USER 0:0
VOLUME ["/data"]
EXPOSE 4433/udp
ENTRYPOINT ["/usr/local/bin/nife-remote-server"]

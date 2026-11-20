# syntax=docker/dockerfile:1
# ── 构建 ──────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY server ./server
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p airhand-server \
 && cp target/release/airhand-server /airhand-server

# ── 运行（distroless，无 shell，以 root 运行）────────────
# 以 root 运行（不用 :nonroot 标签、不设 USER）：挂载进来的 /data 不管属于谁都写得进去。目录不存在时 Docker 替你建的属于 root，
# 自己 mkdir 的属于你的用户，nonroot（65532）都写不进去，服务会因为写不了证书而退出。代价：data/ 里生成的文件在宿主机上属于 root。
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /airhand-server /usr/local/bin/airhand-server
ENV AIRHAND_LISTEN=0.0.0.0:4433 \
    AIRHAND_CERT=/data/cert.pem \
    AIRHAND_KEY=/data/key.pem \
    RUST_LOG=info
VOLUME ["/data"]
EXPOSE 4433/udp
ENTRYPOINT ["/usr/local/bin/airhand-server"]

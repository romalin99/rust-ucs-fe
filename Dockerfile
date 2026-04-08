# ===== 第一阶段：编译 =====
FROM rust:1.94-slim AS builder

# 安装系统依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# 先复制依赖文件（利用Docker缓存）
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src

# 复制源码并编译
COPY src ./src
COPY config ./config
RUN touch src/main.rs && cargo build --release --locked

# ===== 第二阶段：运行环境（最小化） =====
FROM debian:bookworm-slim AS runtime

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    libssl3 \
    libpq5 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# 创建非root用户（安全）
RUN useradd -m -u 1000 -s /bin/bash appuser
USER appuser
WORKDIR /home/appuser

# 从builder复制二进制文件
COPY --from=builder /usr/src/app/target/release/my_project ./
COPY --from=builder /usr/src/app/config ./config

# 暴露端口
EXPOSE 7001

# 环境变量
ENV APP_ENV=production \
    RUST_LOG=info

# 健康检查
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# 启动命令
CMD ["./ucs-fe"]

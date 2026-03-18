# rust-ucs-fe

`tcg-ucs-fe` 的 Rust 移植版，使用 [axum](https://github.com/tokio-rs/axum) 框架实现玩家自助密码重置（SSPR）身份验证服务。

---

## 技术栈

| 组件 | 库 |
|------|-----|
| HTTP 框架 | [axum 0.8](https://github.com/tokio-rs/axum) |
| 异步运行时 | tokio 1.x (multi-thread) |
| Oracle DB | `oracle 0.6` + `r2d2`（`spawn_blocking` 包装） |
| Redis | `redis 0.27` + 哨兵模式 |
| HTTP 客户端 | `reqwest 0.12` (rustls-tls) |
| 序列化 | `serde` + `serde_json` |
| 配置 | `config 0.14` (TOML) |
| 日志 | `tracing` + `tracing-subscriber` (JSON / 文本) |
| 指标 | `metrics` + `metrics-exporter-prometheus` |
| 错误处理 | `thiserror 2` |

---

## 项目结构

```
rust-ucs-fe/
├── Cargo.toml
├── config/
│   ├── dev.toml          # 开发环境
│   ├── sit.toml          # SIT 环境
│   └── prod.toml         # 生产环境
└── src/
    ├── main.rs           # 入口、依赖组装、优雅关闭
    ├── config.rs         # 配置结构体
    ├── error.rs          # AppError / ServiceError / InfraError
    ├── state.rs          # axum AppState（共享依赖）
    ├── router.rs         # axum Router 注册
    ├── db/
    │   └── mod.rs        # Oracle r2d2 连接池 + spawn_blocking 帮助函数
    ├── model/
    │   ├── merchant_rule.rs      # MerchantRule / Question / QuestionInfo
    │   └── validation_record.rs  # ValidationRecord / QA
    ├── repository/
    │   ├── merchant_rule.rs      # Oracle 商户规则数据访问
    │   └── validation_record.rs  # Oracle 验证记录 MERGE UPSERT
    ├── client/
    │   ├── uss.rs         # USS HTTP 客户端 + 模型
    │   └── mcs.rs         # MCS HTTP 客户端 + 模型
    ├── service/
    │   ├── verification.rs  # 核心业务逻辑（限流 / 评分 / SSPR）
    │   └── field_cache.rs   # DashMap 字段配置缓存
    ├── handler/
    │   ├── mod.rs           # ping / liveness / readiness
    │   └── verification.rs  # GetQuestionList + SubmitVerifyMaterials
    └── middleware/
        ├── logger.rs        # 请求行为日志（axum middleware）
        └── metrics.rs       # Prometheus 指标采集
```

---

## 接口

| 方法 | 路径 | 说明 |
|------|------|------|
| GET  | `/tcg-ucs-fe/ping`                    | 存活探针 |
| GET  | `/tcg-ucs-fe/verification/questions`  | 获取验证问题列表 |
| POST | `/tcg-ucs-fe/verification/materials`  | 提交验证材料 |
| GET  | `/livez`                              | Liveness 健康检查 |
| GET  | `/readyz`                             | Readiness 健康检查 |
| GET  | `/metrics`                            | Prometheus 指标 |

---

## 快速启动

### 前置条件

- Rust 1.80+（`rustup update stable`）
- Oracle Instant Client（`oracle` crate 需要 OCI 库）
  - macOS: `brew install instantclient-basic`
  - Linux: 下载 [Oracle Instant Client](https://www.oracle.com/database/technologies/instant-client.html)
- Redis Sentinel（或单节点 Redis 用于本地开发）
- AWS 凭据（用于 Secrets Manager，可通过配置文件直接填写凭据跳过）

### 环境变量（可选覆盖）

```bash
export APP__ORACLE__USER=myuser
export APP__ORACLE__PASSWORD=mypassword
export APP__ORACLE__CONNECT_STRING=localhost:1521/XE
export RUST_LOG=info
```

### 编译并运行

```bash
cd rust-ucs-fe

# 开发模式
cargo run -- -f ./config/dev.toml

# 发布构建
cargo build --release
./target/release/ucs-fe -f ./config/prod.toml
```

---

## 与 Go 版本的对比

| Go | Rust |
|----|------|
| `fiber.App` | `axum::Router` |
| `fiber.Ctx` | `axum::extract::*` |
| `c.Next()` 中间件链 | `axum::middleware::from_fn` |
| `sync.Once` 全局单例 | `Arc<T>` 注入 |
| `goroutine` | `tokio::spawn` |
| `oracle` (godror) | `oracle` crate + `spawn_blocking` |
| `go-redis` | `redis` crate |
| `prometheus/client_golang` | `metrics-exporter-prometheus` |
| `zap` | `tracing` + `tracing-subscriber` |
| `errors.Is` | `thiserror` + `match` |
| `context.WithTimeout` | `tokio::time::timeout` |

---

## 架构说明

### Oracle 异步集成

`oracle` crate 是同步驱动。所有 Oracle 操作通过 `tokio::task::spawn_blocking` 在专用线程池中执行，不阻塞 tokio 的异步 I/O 线程：

```rust
pub async fn run<F, R>(pool: &OraclePool, f: F) -> Result<R, AppError>
where
    F: FnOnce(PooledConnection<OracleManager>) -> Result<R, oracle::Error> + Send + 'static,
    R: Send + 'static,
{ ... }
```

### Redis 原子限流

使用与 Go 版本完全相同的 Lua 脚本，保证 `INCR + EXPIREAT` 原子性：

```lua
local v = redis.call('INCR', KEYS[1])
if v == 1 then
    redis.call('EXPIREAT', KEYS[1], ARGV[1])
end
return v
```

### 错误处理分层

```
InfraError (Oracle/Redis/Http/Json/Pool)
    ↓ wraps into
ServiceError (MerchantNotFound/LimitExceeded/...)
    ↓ wraps into
AppError → impl IntoResponse → HTTP status + JSON body
```

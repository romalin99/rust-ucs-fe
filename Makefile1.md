# Makefile.toml — ucs-fe 开发任务手册

> 基于 [cargo-make](https://github.com/sagiegurari/cargo-make) 的任务配置文件。  
> 所有命令均以 `cargo make <task>` 方式执行。

---

## 目录

- [环境要求](#环境要求)
- [快速开始](#快速开始)
- [开发流程图](#开发流程图)
- [任务依赖关系](#任务依赖关系)
- [命令参考](#命令参考)
  - [初始化](#初始化)
  - [构建](#构建)
  - [运行](#运行)
  - [代码质量](#代码质量)
  - [测试](#测试)
  - [文档](#文档)
  - [安全与依赖](#安全与依赖)
  - [数据库迁移](#数据库迁移)
  - [Watch 热重载](#watch-热重载)
  - [清理](#清理)
  - [CI 流水线](#ci-流水线)
  - [Docker](#docker)
  - [统计](#统计)
- [配置说明](#配置说明)
- [规范与注意事项](#规范与注意事项)

---

## 环境要求

| 工具 | 说明 |
|------|------|
| Rust stable | 主编译工具链 |
| cargo-make ≥ 0.37.0 | 任务运行器本体 |
| cargo-watch | watch 系列任务 |
| cargo-audit | CVE 检查 |
| cargo-deny | 供应链安全 |
| cargo-tarpaulin | 覆盖率报告 |
| cargo-nextest | 更快的测试运行器 |
| cargo-edit | `cargo upgrade` 支持 |
| cargo-outdated | 依赖过时检查 |
| cargo-udeps | 未使用依赖检查（需 nightly）|
| sqlx-cli | 数据库迁移管理 |
| tokei | 代码行数统计 |

一键安装所有工具：

```bash
cargo make install
```

---

## 快速开始

```bash
# 1. 初始化开发环境（仅首次）
cargo make install

# 2. 默认任务：格式检查 + debug 构建
cargo make

# 3. 启动开发服务
cargo make run-dev

# 4. 提交前验证
cargo make ci
```

---

## 开发流程图

下图展示了从项目初始化到发版部署的完整命令使用次序：

<div align="center">

![开发流程图](./workflow.svg)

</div>

**各阶段说明：**

- **阶段 1（初始化）**：项目首次克隆后执行一次，安装所有开发工具并生成安全配置模板。
- **阶段 2（日常迭代）**：每次修改代码时按 `fmt → check → clippy → test → run-dev` 顺序执行，可用 `watch-run` 代替手动重启。
- **阶段 3（提交前）**：`optimize` 自动修复后 `ci` 做最终验证，两者可按需组合或单独运行。
- **阶段 4（发版/CI）**：流水线中用 `ci-full` 替代阶段 3，然后构建 release 包并打 Docker 镜像。

---

## 任务依赖关系

下图展示了主要复合任务的依赖展开：

<div align="center">

![任务依赖关系](./task_deps.svg)

</div>

| 任务 | 依赖链 | 说明 |
|------|--------|------|
| `ci` | fmt-check → clippy → test | 本地提交前标准门控 |
| `ci-security` | audit → deny | 安全专项检查 |
| `security` | — | `ci-security` 的别名 |
| `optimize` | clippy-fix → fmt → clippy | 自动修复后验证无残留警告 |
| `ci-full` | fmt-check → clippy → test → audit → deny → doc-check | CI 流水线完整门控 |
| `default` | fmt-check → build | 默认任务（无参数时执行） |

---

## 命令参考

### 初始化

```bash
cargo make install          # 安装所有推荐工具（cargo-watch、cargo-audit 等）
cargo make deny-init        # 生成 deny.toml 配置模板（运行一次）
cargo make version          # 查看当前 Rust 工具链版本
```

---

### 构建

```bash
cargo make build            # 编译 debug 二进制（等同于 cargo make）
cargo make debug            # build 的别名
cargo make release          # 编译 release 二进制（--locked，启用优化）
cargo make build-all        # 编译所有 target（bin、examples、tests、benches）
cargo make release-all      # 编译所有 target（release 模式，--locked）
```

> `release` 使用 `--locked` 确保依赖版本与 `Cargo.lock` 严格一致，适合生产构建。

---

### 运行

```bash
cargo make run              # debug 模式运行，ENV 由 shell 决定（默认 dev）
cargo make run-release      # release 模式运行
cargo make run-dev          # 强制 ENV=dev 运行
cargo make run-sit          # 强制 ENV=sit 运行
cargo make run-prod         # 强制 ENV=prod 运行（⚠ 谨慎使用）
```

**ENV 优先级说明：**

| 场景 | ENV 值来源 |
|------|-----------|
| `cargo make run` | shell 中导出的 `ENV` 变量（未设置时默认 `dev`）|
| `cargo make run-dev` | 任务内强制设置为 `dev` |
| `cargo make run-sit` | 任务内强制设置为 `sit` |
| `cargo make run-prod` | 任务内强制设置为 `prod` |

> `cargo run` 自身管理增量编译，`run` 系列任务无需额外 `dependencies = ["build"]`。

---

### 代码质量

```bash
cargo make fmt              # 格式化所有源文件（rustfmt）
cargo make fmt-check        # 检查格式是否符合规范（不通过则失败，用于 CI）
cargo make check            # 快速语法检查（无二进制产物，最快）
cargo make clippy           # 运行 Clippy，deny warnings，含 pedantic 检查
cargo make lint             # clippy 的别名
cargo make clippy-fix       # 自动应用 Clippy 修复（best-effort）
cargo make optimize         # clippy-fix → fmt → clippy（修复后验证）
```

**`optimize` 三步说明：**

```
clippy-fix   → 自动修复所有可修复的 Clippy 警告
     ↓
fmt          → 重新格式化（fix 可能引入未对齐代码）
     ↓
clippy       → 验证没有无法自动修复的残留警告
```

**Clippy 启用的检查组：**

| lint 组 | 说明 |
|---------|------|
| `clippy::correctness` | 可能导致错误行为的代码 |
| `clippy::suspicious` | 可疑但不一定错误的写法 |
| `clippy::perf` | 性能相关改进建议 |
| `clippy::complexity` | 过于复杂的表达式 |
| `clippy::style` | 代码风格建议 |
| `clippy::pedantic` | 更严格的附加检查 |

---

### 测试

```bash
cargo make test             # 运行所有测试（--all-targets --locked）
cargo make testv            # 运行测试并显示 stdout 输出（--nocapture）
cargo make test-release     # release 模式运行测试（捕获优化相关 bug）
cargo make nextest          # 使用 cargo-nextest 运行（更快，彩色输出）
cargo make bench            # 运行基准测试
cargo make cover            # 生成 HTML 覆盖率报告（输出至 target/coverage/）
```

覆盖率报告生成后位于：

```
target/coverage/tarpaulin-report.html
```

---

### 文档

```bash
cargo make doc              # 构建 rustdoc 文档（含私有项）并在浏览器打开
cargo make doc-check        # 构建文档但不打开（CI 文档 lint 检查，-D warnings）
```

---

### 安全与依赖

```bash
cargo make audit            # 检查依赖中的已知 CVE（cargo-audit）
cargo make deny             # 运行全部 cargo-deny 检查
cargo make deny-advisories  # 仅检查安全公告和未维护 crate
cargo make deny-bans        # 仅检查禁用 crate 及重复版本
cargo make deny-licenses    # 仅检查许可证合规性（依据 deny.toml）
cargo make deny-sources     # 仅检查 crate 来源是否可信
cargo make ci-security      # audit + deny（安全 CI 门控）
cargo make security         # ci-security 的别名（交互使用）

cargo make outdated         # 列出过时的依赖（-R 递归）
cargo make udeps            # 检测未使用的依赖（需 nightly 工具链）
cargo make update           # 更新 Cargo.lock 到最新兼容版本
cargo make update-toolchain # rustup update + cargo update
```

> ⚠️ **`cargo make upgrade` 注意事项**
>
> 此命令使用 `--incompatible` 标志，会将依赖升级到不向后兼容（semver-major）的版本，通常会导致编译失败。  
> **务必在独立分支上执行**，并在合并前完整验证构建和测试。

---

### 数据库迁移

```bash
cargo make migrate          # 运行所有待执行的迁移（sqlx migrate run）
cargo make migrate-revert   # 回滚最近一次迁移（sqlx migrate revert）
```

---

### Watch 热重载

> 需要提前安装 `cargo-watch`：`cargo make install-cargo-watch`

```bash
cargo make watch            # 文件变更时自动重新编译
cargo make watch-run        # 文件变更时自动重编译并重启服务
cargo make watch-test       # 文件变更时自动重跑测试
cargo make watch-check      # 文件变更时自动运行 cargo check（最快反馈）
```

**开发调试推荐工作流：**

```bash
# 终端 1：代码变更时自动重启
cargo make watch-run

# 终端 2：代码变更时自动跑测试
cargo make watch-test
```

---

### 清理

```bash
cargo make clean            # 删除 target/ 目录
cargo make dist-clean       # 删除 target/ + Cargo.lock（完全重置）
```

> `dist-clean` 使用平台专属脚本：Linux/macOS 使用 `rm -f`，Windows 使用 `del /f`，跨平台兼容。

---

### CI 流水线

```bash
cargo make ci               # 基础门控：fmt-check → clippy → test
cargo make ci-security      # 安全门控：audit + deny
cargo make ci-full          # 完整门控：fmt-check → clippy → test → audit → deny → doc-check
```

**CI 场景选择建议：**

| 场景 | 推荐命令 |
|------|---------|
| 每次 push（快速反馈）| `cargo make ci` |
| PR 合并前 | `cargo make ci-full` |
| 定期安全扫描 | `cargo make ci-security` |
| 本地提交前 | `cargo make optimize && cargo make ci` |

> `security` 是 `ci-security` 的交互别名；在 CI pipeline 中建议直接调用 `ci-security`，便于日志识别。

---

### Docker

```bash
cargo make docker-build     # 构建镜像，标签为 ${APP}:${ENV}（默认 ucs-fe:dev）
cargo make docker-run       # 运行容器，映射 8080 端口，加载 .env 文件
```

**示例：构建并运行 production 镜像：**

```bash
ENV=prod cargo make docker-build
ENV=prod cargo make docker-run
```

---

### 统计

```bash
cargo make stats            # 代码行数汇总（按语言分类，自动安装 tokei）
cargo make stats-detail     # 逐文件代码行数明细
cargo make status           # stats 的别名
```

---

## 配置说明

### 全局环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `APP` | `ucs-fe` | 二进制名称，与 `Cargo.toml` 中的 `[[bin]] name` 一致 |
| `ENV` | `dev` | 运行环境标识，仅在 shell 未设置时生效 |

**在 shell 中覆盖 ENV：**

```bash
# 临时覆盖
ENV=sit cargo make run

# 或通过 run-sit 任务（效果相同）
cargo make run-sit
```

### cargo-make 版本要求

```toml
[config]
min_version = "0.37.0"
```

若版本不满足，cargo-make 会在启动时报错。

---

## 规范与注意事项

### 设计原则

1. **`cargo run` 自管理编译**：`run`、`run-release` 等任务不声明 `dependencies = ["build"]`，避免 cargo 执行重复编译检查。

2. **ENV 变量不自赋值**：全局 `[env]` 已通过 `condition = { env_not_set = ["ENV"] }` 设置默认值，任务级别无需重复赋值 `ENV = "${ENV}"`。

3. **`optimize` 末尾 clippy 是校验步骤**：`clippy-fix` 只能修复可自动修复的问题，最后一次 `clippy` 用于捕获无法自动修复的残留警告，确保代码在提交前干净。

4. **`security` 是别名而非独立任务**：在 CI 日志中应看到 `ci-security` 字样而非 `security`，命名更明确。

5. **`dist-clean` 跨平台兼容**：通过 `[tasks.dist-clean.linux]` 和 `[tasks.dist-clean.windows]` 分别处理，不依赖 Unix-only 命令。

6. **`watch-run` 使用 `${APP}` 变量**：cargo-make 在将 `args` 传递给进程前会展开 `[env]` 变量，因此 `watch-run` 中的 `${APP}` 可以正确解析，同时保持与全局 `APP` 配置同步。

### 常见问题

**Q：`cargo make` 默认执行什么？**  
A：执行 `default` 任务：`fmt-check → build`。若格式不符合规范，会在 `fmt-check` 步骤失败。

**Q：`cargo make clippy` 和 `cargo make clippy-fix` 有什么区别？**  
A：`clippy` 只报告警告并在有警告时失败；`clippy-fix` 会尝试自动修改源文件修复警告，使用 `--allow-dirty` 允许工作目录有未提交改动。

**Q：`cargo make test` 和 `cargo make nextest` 如何选择？**  
A：`nextest` 更快（并行执行）且输出更友好，推荐日常使用；`test` 是标准 cargo test，兼容性更好，用于 CI。

**Q：`cargo make upgrade` 执行后编译失败怎么办？**  
A：这是预期行为。`--incompatible` 会引入 breaking change，需要手动更新代码适配新 API，完成后再合并到主分支。

**Q：`help` 命令输出内容太多怎么办？**  
A：`cargo make help` 等同于 `cargo make --list-all-steps`，会列出所有任务及其描述。可配合 `grep` 过滤：
```bash
cargo make help | grep ci
```

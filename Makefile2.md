# ucs-fe — Makefile.toml 分析与使用手册

> 基于 [cargo-make](https://github.com/sagiegurari/cargo-make) v0.37.0+，管理 `ucs-fe` 项目的构建、测试、安全审计、Docker 发布全流程。

---

## 一、规范性分析

### ✅ 规范之处

| 维度 | 说明 |
|------|------|
| **结构清晰** | 任务按功能分组，注释完整，`[env]` 集中管理环境变量 |
| **跨平台兼容** | `dist-clean` 使用 `[tasks.dist-clean.linux]` / `[tasks.dist-clean.windows]` 平台覆盖，避免 `rm -f` 在 Windows 失效 |
| **变量一致性** | `APP`、`ENV` 统一用 `[env]` 声明，避免硬编码 |
| **依赖链清晰** | `ci` → `[fmt-check, clippy, test]`，`ci-full` 扩展安全检查，层次分明 |
| **别名设计** | `lint` → `clippy`，`debug` → `build`，`security` → `ci-security`，语义友好 |
| **`--locked` 一致** | `release`、`test`、`release-all` 均带 `--locked`，保证 CI 可重现构建 |
| **watch 系列** | `watch`/`watch-run`/`watch-test`/`watch-check` 覆盖完整热重载场景 |
| **工具自动安装** | `stats`/`stats-detail` 通过 `install_crate = "tokei"` 自动安装，用户体验好 |

### ⚠️ 潜在问题与建议

#### 1. `watch-run` 变量展开风险

```toml
# 当前写法 — cargo-watch 的 -x 参数是字符串传给子命令
args = ["watch", "-x", "run --bin ${APP}"]
```

`cargo-make` 会展开 `${APP}`，但部分版本对内嵌空格的字符串展开行为不稳定。建议改为：

```toml
args = ["watch", "-x", "run --bin ucs-fe"]
# 或使用 script 方式保证跨版本稳定
script = ["cargo watch -x 'run --bin ${APP}'"]
```

#### 2. `upgrade` 任务风险标注不够显眼

文档注释有警告，但任务本身无 `condition` 防护。建议加交互确认或拆分为独立分支任务：

```toml
[tasks.upgrade]
description = "⚠ 升级破坏性版本 — 请在独立分支执行并验证后合并"
```

#### 3. `migrate` / `migrate-revert` 缺少环境保护

生产环境下误执行 `migrate-revert` 风险高。建议增加：

```toml
[tasks.migrate-revert]
condition = { env_not_set = ["ENV"] }   # 或 condition = { env = { ENV = "dev" } }
```

#### 4. `cover` 任务输出目录未在 `.gitignore` 中提示

`target/coverage` 由 tarpaulin 生成，建议在任务描述中注明已排除于版本控制。

#### 5. `help` 任务输出排序

`cargo make --list-all-steps` 输出未排序，可读性较差（原始注释中的 `grep|sort` 方案更佳但跨平台不兼容）。当前实现是可接受的折中，无需修改。

---

## 二、逻辑正确性验证

### 依赖链验证

```
default  →  fmt-check  →  build          ✅ 正确
ci       →  fmt-check → clippy → test    ✅ 正确
ci-full  →  ci 的超集 + audit+deny+doc   ✅ 正确
optimize →  clippy-fix → fmt → clippy    ✅ 正确（先修复再验证）
```

### 环境变量继承

```
全局: ENV=dev (未设置时)
run-dev  → ENV=dev  ✅
run-sit  → ENV=sit  ✅
run-prod → ENV=prod + release 模式  ✅
```

### `--locked` 使用一致性

| 任务 | 是否 `--locked` | 说明 |
|------|----------------|------|
| `build` | ❌ | debug 迭代无需锁定，正确 |
| `release` | ✅ | 生产构建需锁定，正确 |
| `test` | ✅ | CI 测试需可重现，正确 |
| `test-release` | ❌ | 可选加 `--locked`，当前无硬性问题 |

---

## 三、开发工作流与命令次序

### 阶段一：项目初始化（首次）

```bash
# 1. 安装所有推荐工具（cargo-watch, cargo-audit, sqlx-cli 等）
cargo make install

# 2. 确认工具链版本
cargo make version
```

### 阶段二：日常开发循环

```
代码修改 → 格式化 → 快速检查 → lint → 测试 → 运行
```

```bash
# 方式 A：手动逐步（推荐初学者）
cargo make fmt          # 格式化代码
cargo make check        # 快速语法检查（无代码生成，最快）
cargo make clippy       # lint（含 pedantic 警告）
cargo make test         # 运行所有测试

# 方式 B：默认命令（fmt-check + build，适合提交前快速确认）
cargo make

# 方式 C：热重载开发（推荐日常使用）
cargo make watch-run    # 文件变化时自动重编译并重启
cargo make watch-test   # 文件变化时自动重跑测试
cargo make watch-check  # 最快反馈，只检查不编译
```

**推荐日常节奏：**

```bash
# 终端 1 — 热重载运行
cargo make watch-run

# 终端 2 — 热重载测试
cargo make watch-test
```

### 阶段三：代码提交前

```bash
# 自动修复 Clippy 问题 → 重新格式化 → 验证无残余警告
cargo make optimize

# 或手动逐步
cargo make fmt-check    # 确认格式
cargo make clippy       # 确认无警告
cargo make test         # 确认测试通过
```

### 阶段四：数据库迁移（如有）

```bash
# 应用待执行的迁移
cargo make migrate

# 回滚最近一次迁移（仅在 dev 环境操作！）
cargo make migrate-revert
```

### 阶段五：CI 门控（推送前 / PR 前）

```bash
# 基础门控（快）：格式 + lint + 测试
cargo make ci

# 安全门控：CVE 扫描 + 供应链检查
cargo make ci-security   # 或 cargo make security

# 完整门控（慢，合并主干前使用）
cargo make ci-full       # fmt-check + clippy + test + audit + deny + doc-check
```

### 阶段六：发布

```bash
# 1. 编译 release 二进制
cargo make release

# 2. 构建 Docker 镜像（默认 ENV=dev，可 export ENV=prod 后执行）
export ENV=prod
cargo make docker-build   # → 镜像 ucs-fe:prod

# 3. 本地验证镜像
cargo make docker-run     # → 8080 端口

# 4. 按 prod 环境运行（非 Docker）
cargo make run-prod
```

---

## 四、完整命令速查表

### 构建

| 命令 | 说明 |
|------|------|
| `cargo make` | 默认：fmt-check + debug build |
| `cargo make build` | debug 编译 |
| `cargo make release` | release 编译（`--locked`） |
| `cargo make build-all` | 编译所有 target |
| `cargo make check` | 快速语法检查 |

### 运行

| 命令 | 说明 |
|------|------|
| `cargo make run` | debug 模式运行 |
| `cargo make run-dev` | ENV=dev |
| `cargo make run-sit` | ENV=sit |
| `cargo make run-prod` | ENV=prod + release |

### 代码质量

| 命令 | 说明 |
|------|------|
| `cargo make fmt` | 格式化 |
| `cargo make fmt-check` | 检查格式（CI 用） |
| `cargo make clippy` | Lint（deny warnings） |
| `cargo make clippy-fix` | 自动修复 Lint |
| `cargo make optimize` | 修复 → 格式化 → 验证 |

### 测试

| 命令 | 说明 |
|------|------|
| `cargo make test` | 所有测试 |
| `cargo make testv` | 带输出 |
| `cargo make nextest` | 使用 cargo-nextest |
| `cargo make cover` | HTML 覆盖率报告 |
| `cargo make bench` | 基准测试 |

### 热重载

| 命令 | 说明 |
|------|------|
| `cargo make watch-run` | 修改即重启 |
| `cargo make watch-test` | 修改即测试 |
| `cargo make watch-check` | 修改即检查 |

### 安全与依赖

| 命令 | 说明 |
|------|------|
| `cargo make audit` | CVE 扫描 |
| `cargo make deny` | 完整供应链检查 |
| `cargo make outdated` | 过时依赖列表 |
| `cargo make udeps` | 未使用依赖（nightly） |
| `cargo make update` | 更新 Cargo.lock |
| `cargo make upgrade` | ⚠ 升级含破坏性版本 |

### CI

| 命令 | 说明 |
|------|------|
| `cargo make ci` | fmt-check + clippy + test |
| `cargo make ci-security` | audit + deny |
| `cargo make ci-full` | 完整门控 |

### Docker

| 命令 | 说明 |
|------|------|
| `cargo make docker-build` | 构建镜像 `APP:ENV` |
| `cargo make docker-run` | 运行容器（8080） |

### 工具维护

| 命令 | 说明 |
|------|------|
| `cargo make install` | 安装推荐工具 |
| `cargo make version` | 查看工具链版本 |
| `cargo make stats` | 代码行数统计 |
| `cargo make clean` | 删除 target/ |
| `cargo make dist-clean` | 删除 target/ + Cargo.lock |
| `cargo make doc` | 构建并打开文档 |
| `cargo make migrate` | 执行数据库迁移 |

---

## 五、环境变量说明

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `APP` | `ucs-fe` | 二进制名称 |
| `ENV` | `dev` | 运行环境（dev/sit/prod） |

覆盖方式：

```bash
# Shell 级别覆盖
export ENV=sit
cargo make run

# 或单次覆盖
ENV=sit cargo make run
```

---

*文档基于 `Makefile.toml`（cargo-make ≥ 0.37.0）生成，最后更新：2026-04*

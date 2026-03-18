#!/usr/bin/env bash
# Rust 全工具链自动维护 & 更新脚本（stable + nightly）
# 使用建议：每周或每月执行一次，可放入 crontab
# 示例：./rust-upgrade.sh [--dry-run]

export CARGO_NET_GIT_FETCH_WITH_CLI=true
set -uo pipefail

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" || "${1:-}" == "-n" ]]; then
    DRY_RUN=true
    echo "=== 干跑模式（--dry-run）开启，仅显示将要执行的命令，不实际运行 ==="
    echo
fi

run() {
    if $DRY_RUN; then
        echo " [DRY] $*"
    else
        echo " 执行: $*"
        "$@"
    fi
}

section() {
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo " $1"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

# ────────────────────────────────────────────────
section "Rust 环境自动升级开始 - $(date '+%Y-%m-%d %H:%M:%S %Z')"
# ────────────────────────────────────────────────

if ! command -v rustup &> /dev/null; then
    echo "错误：未找到 rustup 命令，请先安装 Rust（https://rustup.rs）"
    exit 1
fi

# ────────────────────────────────────────────────
section "1. 更新 rustup 自身"
# ────────────────────────────────────────────────
run rustup self update

# ────────────────────────────────────────────────
section "2. 更新 stable toolchain"
# ────────────────────────────────────────────────
run rustup update stable

# ────────────────────────────────────────────────
section "3. 更新 nightly toolchain"
# ────────────────────────────────────────────────
run rustup update nightly

# ────────────────────────────────────────────────
section "4. 安装/更新常用组件（stable）"
# ────────────────────────────────────────────────
stable_components=(
    rustfmt
    clippy
    rust-docs
)

for comp in "${stable_components[@]}"; do
    run rustup component add "$comp" --toolchain stable || {
        echo " 警告：stable 的 $comp 添加/更新失败（可能已存在或网络问题）"
    }
done

# ────────────────────────────────────────────────
section "5. 安装/更新常用组件（nightly）"
# ────────────────────────────────────────────────
nightly_components=(
    rust-src
    rust-analyzer
    clippy
    rustfmt
    llvm-tools-preview      # cargo-llvm-cov、cargo-flamegraph 依赖
    miri                    # UB 检查（可选）
    # rls                  # 旧版，已基本被 rust-analyzer 取代
)

for comp in "${nightly_components[@]}"; do
    run rustup +nightly component add "$comp" || {
        echo " 警告：nightly 的 $comp 添加/更新失败（可能已存在或网络问题）"
    }
done

# ────────────────────────────────────────────────
section "6. 更新 cargo 工具"
# ────────────────────────────────────────────────

# 安装 cargo-update（批量更新 cargo install 的工具）
if ! command -v cargo-install-update &> /dev/null; then
    echo " 安装 cargo-update ..."
    run cargo install cargo-update --locked || {
        echo " 警告：cargo-update 安装失败，继续执行其他步骤"
    }
fi

# 使用 cargo install-update 更新所有已安装的 cargo 工具
if command -v cargo-install-update &> /dev/null; then
    echo " 更新所有 cargo 工具 (locked 模式)..."
    run cargo install-update -a --locked --jobs 10 || {
        echo " 警告：部分 cargo 工具更新失败，继续..."
    }
    echo ""
    echo " 清理旧构建缓存..."
    run cargo cache -a || echo " 警告：cargo cache 清理失败（可忽略）"
else
    echo " 未找到 cargo-install-update，跳过 cargo 工具批量更新"
fi

# ────────────── 常用高质量 cargo 子命令工具 ──────────────
echo ""
echo " 安装/更新常用 cargo 子命令工具..."

# cargo-nextest ── 快速、隔离的测试运行器
if ! command -v cargo-nextest &> /dev/null; then
    echo "   → 首次安装 cargo-nextest"
    run cargo install cargo-nextest --locked || echo "   警告：cargo-nextest 安装失败"
else
    echo "   → 更新 cargo-nextest"
    run cargo install-update cargo-nextest --locked || echo "   警告：cargo-nextest 更新失败"
fi

# cargo-deny ── 依赖许可证、漏洞、禁 crate 检查
if ! command -v cargo-deny &> /dev/null; then
    echo "   → 首次安装 cargo-deny"
    run cargo install cargo-deny --locked || echo "   警告：cargo-deny 安装失败"
else
    echo "   → 更新 cargo-deny"
    run cargo install-update cargo-deny --locked || echo "   警告：cargo-deny 更新失败"
fi

# cargo-hack ── 多 feature 组合批量测试
if ! command -v cargo-hack &> /dev/null; then
    echo "   → 首次安装 cargo-hack"
    run cargo install cargo-hack --locked || echo "   警告：cargo-hack 安装失败"
else
    echo "   → 更新 cargo-hack"
    run cargo install-update cargo-hack --locked || echo "   警告：cargo-hack 更新失败"
fi

# cargo-audit ── 依赖已知安全漏洞扫描
if ! command -v cargo-audit &> /dev/null; then
    echo "   → 首次安装 cargo-audit"
    run cargo install cargo-audit --locked || echo "   警告：cargo-audit 安装失败"
else
    echo "   → 更新 cargo-audit"
    run cargo install-update cargo-audit --locked || echo "   警告：cargo-audit 更新失败"
fi

# cargo-outdated ── 检查依赖是否有更新版本
if ! command -v cargo-outdated &> /dev/null; then
    echo "   → 首次安装 cargo-outdated"
    run cargo install cargo-outdated --locked || echo "   警告：cargo-outdated 安装失败"
else
    echo "   → 更新 cargo-outdated"
    run cargo install-update cargo-outdated --locked || echo "   警告：cargo-outdated 更新失败"
fi

# cargo-expand ── 展开宏查看实际生成的代码
if ! command -v cargo-expand &> /dev/null; then
    echo "   → 首次安装 cargo-expand"
    run cargo install cargo-expand --locked || echo "   警告：cargo-expand 安装失败"
else
    echo "   → 更新 cargo-expand"
    run cargo install-update cargo-expand --locked || echo "   警告：cargo-expand 更新失败"
fi

# cargo-udeps ── 检测未使用的依赖
if ! command -v cargo-udeps &> /dev/null; then
    echo "   → 首次安装 cargo-udeps"
    run cargo install cargo-udeps --locked || echo "   警告：cargo-udeps 安装失败"
else
    echo "   → 更新 cargo-udeps"
    run cargo install-update cargo-udeps --locked || echo "   警告：cargo-udeps 更新失败"
fi

# cargo-watch ── 文件变更监视 + 自动执行 cargo 命令（热重载开发）
if ! command -v cargo-watch &> /dev/null; then
    echo "   → 首次安装 cargo-watch"
    run cargo install cargo-watch --locked || echo "   警告：cargo-watch 安装失败"
else
    echo "   → 更新 cargo-watch"
    run cargo install-update cargo-watch --locked || echo "   警告：cargo-watch 更新失败"
fi

# bacon ── cargo-watch 的现代替代品（更稳定、UI 更好，推荐使用）
if ! command -v bacon &> /dev/null; then
    echo "   → 首次安装 bacon（cargo-watch 现代替代）"
    run cargo install bacon --locked || echo "   警告：bacon 安装失败"
else
    echo "   → 更新 bacon"
    run cargo install-update bacon --locked || echo "   警告：bacon 更新失败"
fi

# cargo-tarpaulin ── 代码覆盖率报告生成工具
if ! command -v cargo-tarpaulin &> /dev/null; then
    echo "   → 首次安装 cargo-tarpaulin"
    run cargo install cargo-tarpaulin --locked || echo "   警告：cargo-tarpaulin 安装失败"
else
    echo "   → 更新 cargo-tarpaulin"
    run cargo install-update cargo-tarpaulin --locked || echo "   警告：cargo-tarpaulin 更新失败"
fi

# cargo-binstall ── 快速安装预编译二进制 crate（加速 cargo install）
if ! command -v cargo-binstall &> /dev/null; then
    echo "   → 首次安装 cargo-binstall"
    run cargo install cargo-binstall --locked || echo "   警告：cargo-binstall 安装失败"
else
    echo "   → 更新 cargo-binstall"
    run cargo install-update cargo-binstall --locked || echo "   警告：cargo-binstall 更新失败"
fi

# cargo-cranky ── 极严格的 Clippy lint 检查（代码质量门禁）
if ! command -v cargo-cranky &> /dev/null; then
    echo "   → 首次安装 cargo-cranky"
    run cargo install cargo-cranky --locked || echo "   警告：cargo-cranky 安装失败"
else
    echo "   → 更新 cargo-cranky"
    run cargo install-update cargo-cranky --locked || echo "   警告：cargo-cranky 更新失败"
fi

# cargo-make ── Rust-aware 的任务运行器 / 构建脚本工具（类似 make）
if ! command -v cargo-make &> /dev/null; then
    echo "   → 首次安装 cargo-make"
    run cargo install cargo-make --locked || echo "   警告：cargo-make 安装失败"
else
    echo "   → 更新 cargo-make"
    run cargo install-update cargo-make --locked || echo "   警告：cargo-make 更新失败"
fi

# cargo-dist ── 多平台二进制发布 + 安装器生成工具
if ! command -v cargo-dist &> /dev/null; then
    echo "   → 首次安装 cargo-dist"
    run cargo install cargo-dist --locked || echo "   警告：cargo-dist 安装失败"
else
    echo "   → 更新 cargo-dist"
    run cargo install-update cargo-dist --locked || echo "   警告：cargo-dist 更新失败"
fi

# 已有的性能/覆盖率工具
if ! command -v cargo-llvm-cov &> /dev/null; then
    echo "   → 首次安装 cargo-llvm-cov"
    run cargo install cargo-llvm-cov --locked || echo "   警告：cargo-llvm-cov 安装失败"
else
    echo "   → 更新 cargo-llvm-cov"
    run cargo install-update cargo-llvm-cov --locked || echo "   警告：cargo-llvm-cov 更新失败"
fi

if ! command -v cargo-flamegraph &> /dev/null; then
    echo "   → 首次安装 cargo-flamegraph"
    run cargo install cargo-flamegraph --locked || echo "   警告：cargo-flamegraph 安装失败"
else
    echo "   → 更新 cargo-flamegraph"
    run cargo install-update cargo-flamegraph --locked || echo "   警告：cargo-flamegraph 更新失败"
fi

# 可选：更多工具（根据需要自行打开）
# run cargo install --locked cargo-watch cargo-tarpaulin cargo-binstall cargo-cranky || true

# ────────────────────────────────────────────────
section "7. 清理旧的 toolchain（可选）"
# ────────────────────────────────────────────────
# 取消下面注释即可启用自动清理超过 90 天的旧版本
# echo " 清理超过 90 天的旧 toolchain..."
# run rustup toolchain list | grep -v stable | grep -v nightly | xargs -n1 -I{} rustup toolchain uninstall {} || true

# ────────────────────────────────────────────────
section "当前 Rust 环境状态"
# ────────────────────────────────────────────────
echo "Rustup 版本:"
run rustup --version

echo ""
echo "默认 toolchain:"
run rustup default

echo ""
echo "已安装 toolchain 列表:"
run rustup toolchain list

echo ""
echo "stable 组件:"
run rustup +stable component list --installed

echo ""
echo "nightly 组件:"
run rustup +nightly component list --installed

# ────────────────────────────────────────────────
section "Rust 全工具链升级完成 - $(date '+%Y-%m-%d %H:%M:%S %Z')"
# ────────────────────────────────────────────────

echo "建议：定期运行此脚本（例如每周一次）"
echo "如需干跑测试： $0 --dry-run"
echo ""


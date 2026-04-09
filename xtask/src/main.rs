// ─────────────────────────────────────────────────────────────────────────────
// xtask — cargo xtask task runner for ucs-fe
//
// Replaces Makefile.toml (cargo-make) with a pure-Rust task runner invoked
// via `cargo xtask <command>`.
//
// Usage:
// ── DEFAULT ──────────────────────────────────────────────────────────────
// cargo xtask                  # fmt-check → debug build (default)
// cargo xtask --help           # list all available commands
//
// ── BUILD ────────────────────────────────────────────────────────────────
// cargo xtask build            # compile debug binary
// cargo xtask debug            # alias for build
// cargo xtask release          # compile release binary (--locked)
// cargo xtask build-all        # compile ALL targets (debug)
// cargo xtask release-all      # compile ALL targets (release, --locked)
//
// ── RUN ──────────────────────────────────────────────────────────────────
// cargo xtask run              # run debug binary (ENV=dev)
// cargo xtask run-release      # run release binary
// cargo xtask run-dev          # run with ENV=dev
// cargo xtask run-sit          # run with ENV=sit
// cargo xtask run-prod         # run with ENV=prod (caution)
//
// ── CODE QUALITY ─────────────────────────────────────────────────────────
// cargo xtask fmt              # format all source files
// cargo xtask fmt-check        # check formatting (CI)
// cargo xtask clippy           # run Clippy linter (deny warnings)
// cargo xtask lint             # alias for clippy
// cargo xtask clippy-fix       # auto-apply Clippy fixes
// cargo xtask optimize         # clippy-fix → fmt → clippy (verify)
// cargo xtask check            # fast cargo check
//
// ── TESTS ────────────────────────────────────────────────────────────────
// cargo xtask test             # run all tests (--all-targets --locked)
// cargo xtask testv            # run tests with stdout visible
// cargo xtask test-release     # run tests in release mode
// cargo xtask nextest          # run tests with cargo-nextest
// cargo xtask bench            # run benchmarks
// cargo xtask cover            # generate HTML coverage report
//
// ── DOCS ─────────────────────────────────────────────────────────────────
// cargo xtask doc              # build docs (private items) + open
// cargo xtask doc-check        # build docs without opening (CI)
//
// ── SECURITY & DEPENDENCIES ──────────────────────────────────────────────
// cargo xtask audit            # check CVEs in dependencies
// cargo xtask deny             # run all cargo-deny checks
// cargo xtask deny-init        # initialize deny.toml template
// cargo xtask deny-advisories  # check for security advisories
// cargo xtask deny-bans        # check for banned crates
// cargo xtask deny-licenses    # check dependency licenses
// cargo xtask deny-sources     # check crate sources
// cargo xtask security         # alias for ci-security (audit + deny)
// cargo xtask outdated         # list outdated dependencies
// cargo xtask udeps            # detect unused dependencies (nightly)
// cargo xtask update           # update Cargo.lock
// cargo xtask upgrade          # upgrade Cargo.toml constraints (⚠ see warning)
// cargo xtask update-toolchain # rustup update + cargo update
//
// ── DATABASE MIGRATIONS ──────────────────────────────────────────────────
// cargo xtask migrate          # run pending migrations (sqlx)
// cargo xtask migrate-revert   # revert last migration (sqlx)
//
// ── WATCH ────────────────────────────────────────────────────────────────
// cargo xtask watch            # recompile on file changes
// cargo xtask watch-run        # recompile + restart on file changes
// cargo xtask watch-test       # re-run tests on file changes
// cargo xtask watch-check      # re-run cargo check on file changes
//
// ── CLEAN ────────────────────────────────────────────────────────────────
// cargo xtask clean            # remove target/
// cargo xtask dist-clean       # remove target/ + Cargo.lock
//
// ── CI ───────────────────────────────────────────────────────────────────
// cargo xtask ci               # fmt-check → clippy → test
// cargo xtask ci-security      # audit + deny
// cargo xtask security         # alias for ci-security
// cargo xtask ci-full          # fmt-check → clippy → test → audit → deny → doc-check
//
// ── DOCKER ───────────────────────────────────────────────────────────────
// cargo xtask docker-build     # docker build -t <APP>:<ENV> .
// cargo xtask docker-run       # docker run -p 8080:8080 --env-file .env
//
// ── STATISTICS ───────────────────────────────────────────────────────────
// cargo xtask stats            # lines-of-code summary
// cargo xtask stats-detail     # per-file lines-of-code
// cargo xtask status           # alias for stats
//
// ── SETUP ────────────────────────────────────────────────────────────────
// cargo xtask install          # install all recommended cargo tools
// cargo xtask version          # print Rust toolchain versions
// ─────────────────────────────────────────────────────────────────────────────

use clap::{Parser, Subcommand};
use std::env;
use std::process::{Command, ExitStatus, Stdio};

const APP: &str = "ucs-fe";

// ─────────────────────────────────────────────────────────────────────────────
// CLI definition
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "ucs-fe task runner (replaces cargo-make)",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    // ── BUILD ────────────────────────────────────────────────────────────────
    /// Compile debug binary (fast iteration build)
    Build,
    /// Alias for `build`
    Debug,
    /// Compile optimised release binary (stripped, fast, locked)
    Release,
    /// Compile ALL targets (bins, examples, tests, benches) in debug mode
    BuildAll,
    /// Compile ALL targets in release mode (locked)
    ReleaseAll,

    // ── RUN ──────────────────────────────────────────────────────────────────
    /// Run the service in debug mode (ENV from shell, default: dev)
    Run,
    /// Run the service in release mode
    RunRelease,
    /// Run with ENV=dev
    RunDev,
    /// Run with ENV=sit
    RunSit,
    /// Run with ENV=prod (release mode — use with caution)
    RunProd,

    // ── CODE QUALITY ─────────────────────────────────────────────────────────
    /// Run `cargo check` (fast, no codegen)
    Check,
    /// Format all source files with rustfmt
    Fmt,
    /// Assert code is already formatted — fails in CI if not
    FmtCheck,
    /// Run Clippy linter (deny warnings)
    Clippy,
    /// Alias for `clippy`
    Lint,
    /// Run Clippy and auto-apply fixes
    ClippyFix,
    /// Auto-fix Clippy → reformat → verify no residual warnings
    Optimize,

    // ── TESTS ────────────────────────────────────────────────────────────────
    /// Run all unit and integration tests (all targets, locked)
    Test,
    /// Run all tests with stdout visible
    Testv,
    /// Run tests in release mode
    TestRelease,
    /// Run tests with cargo-nextest (requires install)
    Nextest,
    /// Run benchmarks
    Bench,
    /// Generate HTML coverage report with tarpaulin
    Cover,

    // ── DOCS ─────────────────────────────────────────────────────────────────
    /// Build rustdoc (including private items) and open in browser
    Doc,
    /// Build docs without opening (CI-safe doc lint check)
    DocCheck,

    // ── SECURITY & DEPENDENCIES ──────────────────────────────────────────────
    /// Check dependencies for known CVEs (requires cargo-audit)
    Audit,
    /// Run all cargo-deny checks
    Deny,
    /// Initialize deny.toml configuration template
    DenyInit,
    /// Check for security advisories
    DenyAdvisories,
    /// Check for banned crates
    DenyBans,
    /// Check dependency licenses
    DenyLicenses,
    /// Check crate sources
    DenySources,
    /// List outdated dependencies
    Outdated,
    /// Detect unused dependencies (requires nightly)
    Udeps,
    /// Update Cargo.lock
    Update,
    /// Upgrade Cargo.toml constraints (including breaking — use on a branch!)
    Upgrade,
    /// Update Rust toolchain + Cargo.lock
    UpdateToolchain,

    // ── DATABASE MIGRATIONS ──────────────────────────────────────────────────
    /// Run pending database migrations (requires sqlx-cli)
    Migrate,
    /// Revert the last applied database migration
    MigrateRevert,

    // ── WATCH ────────────────────────────────────────────────────────────────
    /// Recompile on every source file change (requires cargo-watch)
    Watch,
    /// Recompile and restart on file changes
    WatchRun,
    /// Re-run tests on file changes
    WatchTest,
    /// Re-run `cargo check` on file changes
    WatchCheck,

    // ── CLEAN ────────────────────────────────────────────────────────────────
    /// Remove the `target/` build directory
    Clean,
    /// Remove `target/` AND Cargo.lock (full reset)
    DistClean,

    // ── CI ───────────────────────────────────────────────────────────────────
    /// Basic CI gate: fmt-check → clippy → test
    Ci,
    /// Security-focused CI: audit + deny
    CiSecurity,
    /// Alias for `ci-security`
    Security,
    /// Extended CI gate: fmt-check → clippy → test → audit → deny → doc-check
    CiFull,

    // ── DOCKER ───────────────────────────────────────────────────────────────
    /// Build Docker image tagged as APP:ENV
    DockerBuild,
    /// Run the service inside Docker (port 8080, --env-file .env)
    DockerRun,

    // ── STATISTICS ───────────────────────────────────────────────────────────
    /// Show lines-of-code statistics (requires tokei)
    Stats,
    /// Per-file lines-of-code breakdown
    StatsDetail,
    /// Alias for `stats`
    Status,

    // ── SETUP ────────────────────────────────────────────────────────────────
    /// Install all recommended cargo add-on tools
    Install,
    /// Print Rust toolchain versions
    Version,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Return the project root (parent of the `xtask` directory).
fn project_root() -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be inside the workspace")
        .to_path_buf();
    dir
}

/// Resolve the current ENV value (from env var, default "dev").
fn get_env() -> String {
    env::var("ENV").unwrap_or_else(|_| "dev".into())
}

/// Run a command, inheriting stdio. Panic on failure.
fn run(cmd: &str, args: &[&str]) {
    let status = run_status(cmd, args);
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Run a command, inheriting stdio. Return the ExitStatus.
fn run_status(cmd: &str, args: &[&str]) -> ExitStatus {
    eprintln!("  → {} {}", cmd, args.join(" "));
    Command::new(cmd)
        .args(args)
        .current_dir(project_root())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap_or_else(|e| panic!("failed to execute `{cmd}`: {e}"))
}

/// Run a command with extra env vars.
fn run_with_env(cmd: &str, args: &[&str], envs: &[(&str, &str)]) {
    eprintln!("  → {} {}", cmd, args.join(" "));
    let mut c = Command::new(cmd);
    c.args(args).current_dir(project_root());
    for (k, v) in envs {
        c.env(k, v);
    }
    let status = c
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap_or_else(|e| panic!("failed to execute `{cmd}`: {e}"));
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Install a cargo sub-tool if not already present.
fn cargo_install(krate: &str) {
    run("cargo", &["install", krate, "--locked"]);
}

// ─────────────────────────────────────────────────────────────────────────────
// Task implementations
// ─────────────────────────────────────────────────────────────────────────────

fn task_build() {
    run("cargo", &["build", "--bin", APP]);
}

fn task_release() {
    run("cargo", &["build", "--release", "--bin", APP, "--locked"]);
}

fn task_build_all() {
    run("cargo", &["build", "--all-targets"]);
}

fn task_release_all() {
    run("cargo", &["build", "--release", "--all-targets", "--locked"]);
}

// ── RUN ──────────────────────────────────────────────────────────────────────

fn task_run() {
    run("cargo", &["run", "--bin", APP]);
}

fn task_run_release() {
    run("cargo", &["run", "--release", "--bin", APP]);
}

fn task_run_with_env(env_val: &str, release: bool) {
    let mut args = vec!["run"];
    if release {
        args.push("--release");
    }
    args.extend_from_slice(&["--bin", APP]);
    run_with_env("cargo", &args, &[("ENV", env_val)]);
}

// ── CODE QUALITY ─────────────────────────────────────────────────────────────

fn task_check() {
    run("cargo", &["check", "--all-targets"]);
}

fn task_fmt() {
    run("cargo", &["fmt", "--all"]);
}

fn task_fmt_check() {
    run("cargo", &["fmt", "--all", "--", "--check"]);
}

fn task_clippy() {
    run(
        "cargo",
        &[
            "clippy",
            "--all-targets",
            "--all-features",
            "--",
            "-D", "warnings",
            "-W", "clippy::correctness",
            "-W", "clippy::suspicious",
            "-W", "clippy::perf",
            "-W", "clippy::complexity",
            "-W", "clippy::style",
            "-W", "clippy::pedantic",
        ],
    );
}

fn task_clippy_fix() {
    run(
        "cargo",
        &[
            "clippy",
            "--all-targets",
            "--fix",
            "--allow-dirty",
            "--allow-staged",
            "--",
            "-W", "clippy::pedantic",
        ],
    );
}

fn task_optimize() {
    task_clippy_fix();
    task_fmt();
    task_clippy();
}

// ── TESTS ────────────────────────────────────────────────────────────────────

fn task_test() {
    run("cargo", &["test", "--all-targets", "--locked"]);
}

fn task_testv() {
    run("cargo", &["test", "--all", "--", "--nocapture"]);
}

fn task_test_release() {
    run("cargo", &["test", "--release", "--all"]);
}

fn task_nextest() {
    run("cargo", &["nextest", "run"]);
}

fn task_bench() {
    run("cargo", &["bench"]);
}

fn task_cover() {
    run(
        "cargo",
        &[
            "tarpaulin",
            "--all-features",
            "--workspace",
            "--timeout", "120",
            "--out", "Html",
            "--output-dir", "target/coverage",
        ],
    );
}

// ── DOCS ─────────────────────────────────────────────────────────────────────

fn task_doc() {
    run("cargo", &["doc", "--no-deps", "--open", "--document-private-items"]);
}

fn task_doc_check() {
    run_with_env(
        "cargo",
        &["doc", "--no-deps"],
        &[("RUSTDOCFLAGS", "-D warnings")],
    );
}

// ── SECURITY ─────────────────────────────────────────────────────────────────

fn task_audit() {
    run("cargo", &["audit"]);
}

fn task_deny() {
    run("cargo", &["deny", "check"]);
}

fn task_deny_init() {
    run("cargo", &["deny", "init"]);
}

fn task_deny_advisories() {
    run("cargo", &["deny", "check", "advisories"]);
}

fn task_deny_bans() {
    run("cargo", &["deny", "check", "bans"]);
}

fn task_deny_licenses() {
    run("cargo", &["deny", "check", "licenses"]);
}

fn task_deny_sources() {
    run("cargo", &["deny", "check", "sources"]);
}

fn task_outdated() {
    run("cargo", &["outdated", "-R"]);
}

fn task_udeps() {
    run("cargo", &["+nightly", "udeps", "--all-targets"]);
}

fn task_update() {
    run("cargo", &["update"]);
}

fn task_upgrade() {
    run("cargo", &["upgrade", "--incompatible"]);
    run("cargo", &["update"]);
}

fn task_update_toolchain() {
    run("rustup", &["update"]);
    run("cargo", &["update"]);
}

// ── DATABASE ─────────────────────────────────────────────────────────────────

fn task_migrate() {
    run("sqlx", &["migrate", "run"]);
}

fn task_migrate_revert() {
    run("sqlx", &["migrate", "revert"]);
}

// ── WATCH ────────────────────────────────────────────────────────────────────

fn task_watch() {
    run("cargo", &["watch", "-x", "build"]);
}

fn task_watch_run() {
    run("cargo", &["watch", "-x", "run --bin ucs-fe"]);
}

fn task_watch_test() {
    run("cargo", &["watch", "-x", "test"]);
}

fn task_watch_check() {
    run("cargo", &["watch", "-x", "check"]);
}

// ── CLEAN ────────────────────────────────────────────────────────────────────

fn task_clean() {
    run("cargo", &["clean"]);
}

fn task_dist_clean() {
    task_clean();
    let lock = project_root().join("Cargo.lock");
    if lock.exists() {
        std::fs::remove_file(&lock).expect("failed to remove Cargo.lock");
        eprintln!("  → removed Cargo.lock");
    }
    eprintln!("  dist-clean: target/ and Cargo.lock removed");
}

// ── CI ───────────────────────────────────────────────────────────────────────

fn task_ci() {
    task_fmt_check();
    task_clippy();
    task_test();
}

fn task_ci_security() {
    task_audit();
    task_deny();
}

fn task_ci_full() {
    task_fmt_check();
    task_clippy();
    task_test();
    task_audit();
    task_deny();
    task_doc_check();
}

// ── DOCKER ───────────────────────────────────────────────────────────────────

fn task_docker_build() {
    let env_val = get_env();
    let tag = format!("{APP}:{env_val}");
    run("docker", &["build", "-t", &tag, "."]);
}

fn task_docker_run() {
    let env_val = get_env();
    let tag = format!("{APP}:{env_val}");
    run(
        "docker",
        &["run", "--rm", "-p", "8080:8080", "--env-file", ".env", &tag],
    );
}

// ── STATISTICS ───────────────────────────────────────────────────────────────

fn task_stats() {
    run("tokei", &["--sort", "code"]);
}

fn task_stats_detail() {
    run("tokei", &["--files", "--sort", "code"]);
}

// ── SETUP ────────────────────────────────────────────────────────────────────

fn task_install() {
    let tools = [
        "cargo-watch",
        "cargo-edit",
        "cargo-outdated",
        "cargo-audit",
        "cargo-deny",
        "cargo-tarpaulin",
        "cargo-nextest",
        "cargo-udeps",
        "sqlx-cli",
        "tokei",
    ];
    for tool in &tools {
        cargo_install(tool);
    }
}

fn task_version() {
    run("rustc", &["--version"]);
    run("cargo", &["--version"]);
    run("rustup", &["--version"]);
}

// ─────────────────────────────────────────────────────────────────────────────
// main — dispatch
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    match cli.command {
        // No subcommand → default task (fmt-check → build)
        None => {
            eprintln!("[xtask] default: fmt-check → build");
            task_fmt_check();
            task_build();
        }

        Some(cmd) => match cmd {
            // ── BUILD ────────────────────────────────────────────────────────
            Commands::Build | Commands::Debug => task_build(),
            Commands::Release => task_release(),
            Commands::BuildAll => task_build_all(),
            Commands::ReleaseAll => task_release_all(),

            // ── RUN ──────────────────────────────────────────────────────────
            Commands::Run => task_run(),
            Commands::RunRelease => task_run_release(),
            Commands::RunDev => task_run_with_env("dev", false),
            Commands::RunSit => task_run_with_env("sit", false),
            Commands::RunProd => task_run_with_env("prod", true),

            // ── CODE QUALITY ─────────────────────────────────────────────────
            Commands::Check => task_check(),
            Commands::Fmt => task_fmt(),
            Commands::FmtCheck => task_fmt_check(),
            Commands::Clippy | Commands::Lint => task_clippy(),
            Commands::ClippyFix => task_clippy_fix(),
            Commands::Optimize => task_optimize(),

            // ── TESTS ────────────────────────────────────────────────────────
            Commands::Test => task_test(),
            Commands::Testv => task_testv(),
            Commands::TestRelease => task_test_release(),
            Commands::Nextest => task_nextest(),
            Commands::Bench => task_bench(),
            Commands::Cover => task_cover(),

            // ── DOCS ─────────────────────────────────────────────────────────
            Commands::Doc => task_doc(),
            Commands::DocCheck => task_doc_check(),

            // ── SECURITY ─────────────────────────────────────────────────────
            Commands::Audit => task_audit(),
            Commands::Deny => task_deny(),
            Commands::DenyInit => task_deny_init(),
            Commands::DenyAdvisories => task_deny_advisories(),
            Commands::DenyBans => task_deny_bans(),
            Commands::DenyLicenses => task_deny_licenses(),
            Commands::DenySources => task_deny_sources(),
            Commands::Outdated => task_outdated(),
            Commands::Udeps => task_udeps(),
            Commands::Update => task_update(),
            Commands::Upgrade => task_upgrade(),
            Commands::UpdateToolchain => task_update_toolchain(),

            // ── DATABASE ─────────────────────────────────────────────────────
            Commands::Migrate => task_migrate(),
            Commands::MigrateRevert => task_migrate_revert(),

            // ── WATCH ────────────────────────────────────────────────────────
            Commands::Watch => task_watch(),
            Commands::WatchRun => task_watch_run(),
            Commands::WatchTest => task_watch_test(),
            Commands::WatchCheck => task_watch_check(),

            // ── CLEAN ────────────────────────────────────────────────────────
            Commands::Clean => task_clean(),
            Commands::DistClean => task_dist_clean(),

            // ── CI ───────────────────────────────────────────────────────────
            Commands::Ci => task_ci(),
            Commands::CiSecurity | Commands::Security => task_ci_security(),
            Commands::CiFull => task_ci_full(),

            // ── DOCKER ───────────────────────────────────────────────────────
            Commands::DockerBuild => task_docker_build(),
            Commands::DockerRun => task_docker_run(),

            // ── STATISTICS ───────────────────────────────────────────────────
            Commands::Stats | Commands::Status => task_stats(),
            Commands::StatsDetail => task_stats_detail(),

            // ── SETUP ────────────────────────────────────────────────────────
            Commands::Install => task_install(),
            Commands::Version => task_version(),
        },
    }
}

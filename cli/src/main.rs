//! operon CLI —— 一人公司无服务器框架的命令行工具。
//!
//! 命令：
//! - `gen-seed` / `dev-seed`：Ed25519 密钥生成
//! - `token`：签发测试 JWT
//! - `init --template <api|fullstack|webhook> <name>`：在 apps/ 下脚手架新应用
//! - `dev [--package]`：本地运行（cargo run）
//! - `deploy --env <env> [--yes]`：调 infra/deploy.sh 部署（prod 需确认）
//! - `logs --env <env>`：CloudWatch 流式日志（aws logs tail --follow）

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command as ProcessCmd;

use base64::engine::general_purpose;
use base64::Engine;
use clap::{Parser, Subcommand};
use operon_core::{Jwt, JwtClaims};

#[derive(Parser)]
#[command(name = "operon", about = "一人公司无服务器框架 CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 生成一个 32 字节的 Ed25519 seed（base64 输出）
    GenSeed,
    /// 打印本地开发用的固定 seed（写入 OPERON_DEV_JWT_SEED）
    DevSeed,
    /// 用 seed 签发一个测试 JWT
    Token {
        /// seed（base64，32 字节）
        #[arg(long)]
        seed: String,
        /// 用户 ID（JWT sub）
        #[arg(long)]
        sub: String,
        /// 邮箱（可选）
        #[arg(long)]
        email: Option<String>,
        /// 有效期秒数
        #[arg(long, default_value_t = 86400)]
        ttl: u64,
    },
    /// 在 apps/ 下脚手架新应用
    Init {
        /// 模板：api / fullstack / webhook
        #[arg(long, default_value = "api")]
        template: String,
        /// 项目名（字母数字连字符）
        name: String,
    },
    /// 本地运行（cargo run）
    Dev {
        /// 要运行的 package（默认 operon-site）
        #[arg(long)]
        package: Option<String>,
    },
    /// 部署到 AWS（调 infra/deploy.sh）
    Deploy {
        /// 环境：dev / staging / prod
        #[arg(long, default_value = "dev")]
        env: String,
        /// 生产环境跳过确认
        #[arg(long)]
        yes: bool,
    },
    /// 查看 Lambda 日志（CloudWatch tail）
    Logs {
        /// 环境：dev / staging / prod
        #[arg(long, default_value = "dev")]
        env: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::GenSeed => {
            let seed = random_seed();
            println!("{}", general_purpose::STANDARD.encode(seed));
        }
        Command::DevSeed => {
            // 固定的 dev seed，本地跑通用；也可用 `gen-seed` 换一把。
            let seed = b"0123456789abcdef0123456789abcdef";
            println!("{}", general_purpose::STANDARD.encode(seed));
        }
        Command::Token {
            seed,
            sub,
            email,
            ttl,
        } => {
            let bytes = general_purpose::STANDARD
                .decode(seed.as_bytes())
                .map_err(|e| anyhow::anyhow!("seed 不是合法 base64: {e}"))?;
            let jwt = Jwt::from_seed(&bytes)?;
            let now = operon_core::unix_now();
            let claims = JwtClaims {
                sub,
                email,
                iat: now,
                exp: now + ttl,
                extra: Default::default(),
            };
            println!("{}", jwt.sign(&claims)?);
        }
        Command::Init { template, name } => cmd_init(&template, &name)?,
        Command::Dev { package } => return cmd_dev(package.as_deref()),
        Command::Deploy { env, yes } => return cmd_deploy(&env, yes),
        Command::Logs { env } => return cmd_logs(&env),
    }
    Ok(())
}

fn random_seed() -> [u8; 32] {
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed
}

// ---------- init：脚手架新应用 ----------

fn cmd_init(template: &str, name: &str) -> anyhow::Result<()> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        anyhow::bail!("项目名只能包含字母、数字、连字符");
    }
    let app_dir = Path::new("apps").join(name);
    if app_dir.exists() {
        anyhow::bail!("目录已存在: {}", app_dir.display());
    }
    fs::create_dir_all(app_dir.join("src"))?;

    let cargo_toml = CARGO_TEMPLATE.replace("{name}", name);
    let main_rs = match template {
        "webhook" => WEBHOOK_MAIN,
        "fullstack" => FULLSTACK_MAIN,
        _ => API_MAIN,
    }
    .replace("{name}", name);

    fs::write(app_dir.join("Cargo.toml"), cargo_toml)?;
    fs::write(app_dir.join("src/main.rs"), main_rs)?;
    println!("✅ 已生成 apps/{name}（模板: {template}）");
    println!("下一步：在根 Cargo.toml 的 [workspace].members 加入 \"apps/{name}\"，然后 cargo build");
    Ok(())
}

const CARGO_TEMPLATE: &str = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
operon-core = { path = "../../crates/core" }
operon-dynamo = { path = "../../crates/dynamo" }
axum = "0.8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
"#;

const API_MAIN: &str = r#"//! {name} —— operon 应用（由 `operon init` 生成）
use axum::routing::get;
use axum::{Json, Router};
use operon_core::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_with_setup(|state| async move {
        let router = Router::new()
            .route("/health", get(|| async { Json(serde_json::json!({"status": "ok"})) }))
            .with_state(state)
            .with_operon_defaults();
        Ok(router)
    })
    .await
}
"#;

const FULLSTACK_MAIN: &str = r#"//! {name} —— operon 全栈应用（API + 前端静态托管）
//! 提示：前端文件放 frontend/，由 infra/deploy.sh 上传到 S3。
use axum::routing::get;
use axum::{Json, Router};
use operon_core::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_with_setup(|state| async move {
        let router = Router::new()
            .route("/health", get(|| async { Json(serde_json::json!({"status": "ok"})) }))
            .with_state(state)
            .with_operon_defaults();
        Ok(router)
    })
    .await
}
"#;

const WEBHOOK_MAIN: &str = r#"//! {name} —— operon Webhook 应用（回调签名验证）
//! 提示：实现 operon_core 的 WebhookVerifier，用中间件在业务前验签。
use axum::routing::post;
use axum::Router;
use operon_core::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_with_setup(|state| async move {
        let router = Router::new()
            .route("/webhook", post(|| async { "received" }))
            .with_state(state)
            .with_operon_defaults();
        Ok(router)
    })
    .await
}
"#;

// ---------- dev：本地运行 ----------

fn cmd_dev(package: Option<&str>) -> anyhow::Result<()> {
    let pkg = package.unwrap_or("operon-site");
    println!("cargo run -p {pkg} ...");
    let status = ProcessCmd::new("cargo").args(["run", "-p", pkg]).status()?;
    std::process::exit(status.code().unwrap_or(1));
}

// ---------- deploy：部署 ----------

fn cmd_deploy(env: &str, yes: bool) -> anyhow::Result<()> {
    let script = Path::new("infra").join("deploy.sh");
    if !script.exists() {
        anyhow::bail!("未找到 infra/deploy.sh（请在 operon 仓库根目录运行）");
    }
    if env == "prod" && !yes {
        // 生产环境强制确认（对应文章「部署前强制预览」）
        print!("⚠️  将部署到生产环境 {env}。确认继续？(y/N): ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("已取消");
            return Ok(());
        }
    }
    println!("bash infra/deploy.sh {env}");
    let status = ProcessCmd::new("bash").arg(&script).arg(env).status()?;
    std::process::exit(status.code().unwrap_or(1));
}

// ---------- logs：CloudWatch 流式日志 ----------

fn cmd_logs(env: &str) -> anyhow::Result<()> {
    let group = format!("/aws/lambda/operon-{env}-site");
    println!("aws logs tail {group} --follow ...");
    let status = ProcessCmd::new("aws")
        .args(["logs", "tail", &group, "--follow"])
        .status()?;
    std::process::exit(status.code().unwrap_or(1));
}

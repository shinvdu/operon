//! operon CLI。
//!
//! 对应文章第五节的 CLI 工具（骨架版）。当前提供脚手架与开发所需的基础命令：
//! - `gen-seed`：生成 32 字节 Ed25519 seed（base64），用于初始化 SSM 密钥；
//! - `token`：用指定 seed 签发一个测试 JWT（验证 `/me` 等受保护路由用）；
//! - `dev-seed`：打印本地开发用的固定 seed（`OPERON_DEV_JWT_SEED`）。

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
    /// 生成并打印本地开发用的固定 seed（写入 OPERON_DEV_JWT_SEED）
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
    }
    Ok(())
}

fn random_seed() -> [u8; 32] {
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed
}

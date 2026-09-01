// x402-proxy — stdio MCP server that proxies an upstream HTTP MCP server and
// auto-signs x402 payments (USDC on Base; exact + upto schemes).
//
// Env vars:
//   X402_KEY_REF     — op:// reference to the EVM private key (both commands)
//   X402_MAX_AMOUNT  — per-payment ceiling in decimal USDC (serve; unset = refuse)

use clap::Parser;

use x402_proxy::cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let key_ref = std::env::var("X402_KEY_REF").unwrap_or_default();
    match cli.command {
        Command::Serve { upstream } => x402_proxy::commands::serve::run(&upstream, key_ref).await,
        Command::ApprovePermit2 {
            rpc_url,
            amount,
            yes,
        } => x402_proxy::commands::approve::run(&rpc_url, &amount, yes, &key_ref).await,
    }
}

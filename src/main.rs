// x402-proxy — stdio MCP server that proxies an upstream HTTP MCP server
// and auto-signs x402 payments (USDC on Base, exact scheme).
//
// Env vars:
//   X402_KEY_REF     — op:// reference to the EVM private key (resolved lazily on first payment)
//   X402_MAX_AMOUNT  — per-payment ceiling in decimal USDC (required to sign anything)

use clap::Parser;

#[derive(Parser)]
#[command(name = "x402-proxy", version, about)]
struct Args {
    /// Upstream MCP server URL (e.g. https://mcp.apify.com?payment=x402)
    #[arg(long)]
    upstream: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    eprintln!("[x402-proxy] upstream: {}", args.upstream);
    anyhow::bail!("not yet implemented: proxy bridge")
}

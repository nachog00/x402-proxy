// x402-proxy — stdio MCP server that proxies an upstream HTTP MCP server
// and auto-signs x402 payments (USDC on Base, exact scheme).
//
// Env vars:
//   X402_KEY_REF     — op:// reference to the EVM private key (resolved lazily on first payment)
//   X402_MAX_AMOUNT  — per-payment ceiling in decimal USDC (unset = refuse to sign)

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;

use x402_proxy::key::OpCli;
use x402_proxy::payment::AmountGuard;
use x402_proxy::proxy::X402Proxy;

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

    let key_ref = std::env::var("X402_KEY_REF").unwrap_or_default();
    let guard = match std::env::var("X402_MAX_AMOUNT") {
        Ok(v) => AmountGuard::parse(&v).context("X402_MAX_AMOUNT")?,
        Err(_) => AmountGuard::Unset,
    };
    if matches!(guard, AmountGuard::Unset) {
        eprintln!("[x402-proxy] warning: X402_MAX_AMOUNT unset — all payments will be refused");
    }

    let transport = StreamableHttpClientTransport::from_uri(args.upstream.clone());
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("x402-proxy", env!("CARGO_PKG_VERSION")),
    );
    let upstream = client_info
        .serve(transport)
        .await
        .with_context(|| format!("connecting to upstream {}", args.upstream))?;
    let tools = upstream
        .list_all_tools()
        .await
        .context("listing upstream tools")?;
    eprintln!(
        "[x402-proxy] connected to {} — {} tools",
        args.upstream,
        tools.len()
    );

    let proxy = X402Proxy::new(
        tools,
        upstream.peer().clone(),
        Arc::new(OpCli::new()),
        key_ref,
        guard,
    );

    let service = proxy
        .serve(rmcp::transport::io::stdio())
        .await
        .context("starting stdio MCP server")?;
    eprintln!("[x402-proxy] stdio server ready");
    service.waiting().await?;
    Ok(())
}

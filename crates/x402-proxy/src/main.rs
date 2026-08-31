// x402-proxy — stdio MCP server that proxies an upstream HTTP MCP server and
// auto-signs x402 payments (USDC on Base; exact + upto schemes).
//
// Commands:
//   serve --upstream <url>   proxy an upstream MCP server, signing payments
//   approve-permit2          one-time on-chain USDC.approve(Permit2) for `upto`
//
// Env vars:
//   X402_KEY_REF     — op:// reference to the EVM private key (both commands)
//   X402_MAX_AMOUNT  — per-payment ceiling in decimal USDC (serve; unset = refuse)

use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;

use x402_proxy::approve::ApprovalAmount;
use x402_proxy::key::OpCli;
use x402_proxy::net::HttpUrl;
use x402_proxy::payment::AmountGuard;
use x402_proxy::proxy::X402Proxy;

#[derive(Parser)]
#[command(name = "x402-proxy", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Proxy an upstream MCP server over stdio, auto-signing x402 payments.
    Serve {
        /// Upstream MCP server URL (e.g. https://mcp.apify.com?payment=x402).
        /// Validated as an absolute http(s) URL at parse time.
        #[arg(long)]
        upstream: HttpUrl,
    },
    /// One-time: approve Uniswap Permit2 to spend your USDC on Base. Required
    /// before the `upto` scheme can settle. Broadcasts one tx (costs a little
    /// Base ETH for gas).
    ApprovePermit2 {
        /// Base RPC endpoint used to broadcast the approval.
        #[arg(long, default_value = "https://mainnet.base.org")]
        rpc_url: HttpUrl,
        /// Amount to approve: "max" (default) or decimal USDC like "5".
        /// Validated at parse time.
        #[arg(long, default_value = "max")]
        amount: ApprovalAmount,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let key_ref = std::env::var("X402_KEY_REF").unwrap_or_default();
    match cli.command {
        Command::Serve { upstream } => serve(&upstream, key_ref).await,
        Command::ApprovePermit2 {
            rpc_url,
            amount,
            yes,
        } => x402_proxy::approve::run(&rpc_url, &amount, yes, &key_ref).await,
    }
}

async fn serve(upstream: &HttpUrl, key_ref: String) -> anyhow::Result<()> {
    let guard = match std::env::var("X402_MAX_AMOUNT") {
        Ok(v) => AmountGuard::parse(&v).context("X402_MAX_AMOUNT")?,
        Err(_) => AmountGuard::Unset,
    };
    if matches!(guard, AmountGuard::Unset) {
        eprintln!("[x402-proxy] warning: X402_MAX_AMOUNT unset — all payments will be refused");
    }

    let transport = StreamableHttpClientTransport::from_uri(upstream.as_str().to_string());
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("x402-proxy", env!("CARGO_PKG_VERSION")),
    );
    let upstream_svc = client_info
        .serve(transport)
        .await
        .with_context(|| format!("connecting to upstream {upstream}"))?;
    let tools = upstream_svc
        .list_all_tools()
        .await
        .context("listing upstream tools")?;
    eprintln!(
        "[x402-proxy] connected to {upstream} — {} tools",
        tools.len()
    );

    let proxy = X402Proxy::new(
        tools,
        upstream_svc.peer().clone(),
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

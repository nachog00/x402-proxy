//! Command-line interface definitions.

use clap::{Parser, Subcommand};

use crate::commands::approve::ApprovalAmount;
use crate::net::HttpUrl;

#[derive(Parser)]
#[command(name = "x402-proxy", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
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

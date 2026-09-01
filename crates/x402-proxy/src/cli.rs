//! Command-line interface definitions.

use clap::{Parser, Subcommand, ValueEnum};

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
    /// Register this proxy in front of an upstream MCP server. Prints portable
    /// MCP JSON by default; `--client claude` runs `claude mcp add` for you.
    Install {
        /// Upstream MCP server URL. A bare host (e.g. https://mcp.apify.com)
        /// gets `?payment=x402` appended automatically.
        #[arg(long)]
        upstream: HttpUrl,
        /// Name to register the server under.
        #[arg(long, default_value = "x402")]
        name: String,
        /// Named wallet from the config → `X402_KEY_REF=wallet:<name>` (and its
        /// `max` as the ceiling unless `--max` overrides). Falls back to the
        /// config's `default_wallet` when neither this nor `--key-ref` is given.
        #[arg(long, conflicts_with = "key_ref")]
        wallet: Option<String>,
        /// Explicit key source for `X402_KEY_REF`: op://…, env:VAR, file:/path,
        /// wallet:NAME, or a raw 0x… key. Validated at parse time.
        #[arg(long, conflicts_with = "wallet")]
        key_ref: Option<String>,
        /// Per-payment spend ceiling in decimal USDC. Defaults to the wallet's
        /// `max` when a config wallet is used.
        #[arg(long)]
        max: Option<String>,
        /// Where to install: `print` (portable JSON, default) or `claude`.
        #[arg(long, value_enum, default_value_t = InstallClient::Print)]
        client: InstallClient,
        /// Which config the entry lands in (forwarded to `claude mcp add --scope`;
        /// ignored by `--client print`): `local` (default, this project, private),
        /// `project` (shared via `.mcp.json`), or `user` (all your projects).
        #[arg(long, value_enum, default_value_t = InstallScope::Local)]
        scope: InstallScope,
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

/// Target for `install`: emit portable JSON, or drive a specific client's CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InstallClient {
    /// Print portable `mcpServers` JSON to stdout (paste into any client).
    Print,
    /// Run `claude mcp add` to register the server with Claude.
    Claude,
}

/// Config location for `install --client claude`, mirroring `claude mcp add`'s
/// own `--scope` values so there's one mental model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InstallScope {
    /// Private to you, in this project (claude's default).
    Local,
    /// Shared with the project via a checked-in `.mcp.json`.
    Project,
    /// Available across all your projects.
    User,
}

impl InstallScope {
    /// The value to pass to `claude mcp add --scope`.
    pub fn as_claude_flag(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Project => "project",
            Self::User => "user",
        }
    }
}

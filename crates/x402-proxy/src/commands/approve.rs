//! `approve-permit2` — one-time on-chain setup for the `upto` scheme.
//!
//! Grants Uniswap's Permit2 an allowance to move the wallet's USDC, which
//! Permit2-based `upto` settlement requires. This is the ONLY command that
//! broadcasts a transaction (it costs a little Base ETH for gas). The signing
//! key is resolved via the same `SecretResolver`/`op` path as payments and
//! never leaves this process.

use std::io::Write;

use alloy_network::EthereumWallet;
use alloy_primitives::{Address, U256, address};
use alloy_provider::ProviderBuilder;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::sol;
use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::key::{KeyResolver, SecretResolver};
use crate::net::HttpUrl;
use crate::payment::{AtomicUsdc, BASE_USDC};

/// Canonical Uniswap Permit2 — the spender we approve.
const PERMIT2: Address = address!("000000000022D473030F116dDEE9F6B43aC78BA3");

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 value) external returns (bool);
    }
}

/// The `--amount` flag, parsed at the CLI boundary (parse-don't-validate):
/// either the unlimited `max` approval or an exact USDC ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalAmount {
    Max,
    Exact(AtomicUsdc),
}

impl ApprovalAmount {
    fn to_u256(&self) -> U256 {
        match self {
            Self::Max => U256::MAX,
            Self::Exact(a) => U256::from(a.as_atomic()),
        }
    }
}

impl std::str::FromStr for ApprovalAmount {
    type Err = crate::payment::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("max") {
            Ok(Self::Max)
        } else {
            Ok(Self::Exact(AtomicUsdc::parse_decimal(s)?))
        }
    }
}

impl std::fmt::Display for ApprovalAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Max => write!(f, "max (unlimited)"),
            Self::Exact(a) => write!(f, "{a} USDC"),
        }
    }
}

pub async fn run(
    rpc_url: &HttpUrl,
    amount: &ApprovalAmount,
    yes: bool,
    key_ref: &str,
) -> Result<()> {
    if key_ref.is_empty() {
        bail!("X402_KEY_REF is not set — needed to resolve the signing key");
    }
    let requested = amount.to_u256();

    // Resolve the key (blocking), same multi-source path as payments — honors
    // op://, env:, file:, wallet:, and raw keys. Never logged.
    let config = Config::load().context("loading config")?;
    let key_ref_owned = key_ref.to_string();
    let key = tokio::task::spawn_blocking(move || KeyResolver::new(config).resolve(&key_ref_owned))
        .await
        .context("key resolution task failed")?
        .context("resolving X402_KEY_REF")?;
    let signer: PrivateKeySigner = key
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("resolved key is not a valid EVM private key"))?;
    let owner = signer.address();

    // rpc_url is already a validated http(s) URL (parsed at the CLI boundary).
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(rpc_url.as_url().clone());
    let usdc = IERC20::new(BASE_USDC, &provider);

    // Idempotent: skip if already approved for at least the requested amount.
    let current = usdc
        .allowance(owner, PERMIT2)
        .call()
        .await
        .context("reading current Permit2 allowance")?;
    if current >= requested {
        println!("Permit2 is already approved for {owner} (allowance ≥ requested). Nothing to do.");
        return Ok(());
    }

    println!("Approve Uniswap Permit2 ({PERMIT2}) to spend USDC from {owner} on Base.");
    println!("Amount: {amount}");
    println!("This sends one transaction and costs a little Base ETH for gas.");
    if !yes {
        print!("Proceed? [y/N] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("reading confirmation")?;
        if line.trim() != "y" {
            bail!("aborted");
        }
    }

    let pending = usdc
        .approve(PERMIT2, requested)
        .send()
        .await
        .context("sending approve transaction")?;
    let tx_hash = *pending.tx_hash();
    println!("approve tx sent: {tx_hash} — waiting for confirmation…");
    let receipt = pending
        .get_receipt()
        .await
        .context("waiting for transaction receipt")?;
    if !receipt.status() {
        bail!("approve transaction reverted: https://basescan.org/tx/{tx_hash}");
    }
    println!("✓ Permit2 approved. tx: https://basescan.org/tx/{tx_hash}");
    println!("The wallet is now ready for the x402 `upto` scheme.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<U256, crate::payment::Error> {
        s.parse::<ApprovalAmount>().map(|a| a.to_u256())
    }

    #[test]
    fn parses_max_case_insensitive() {
        assert_eq!(
            "max".parse::<ApprovalAmount>().unwrap(),
            ApprovalAmount::Max
        );
        assert_eq!(
            "MAX".parse::<ApprovalAmount>().unwrap(),
            ApprovalAmount::Max
        );
        assert_eq!(parse("max").unwrap(), U256::MAX);
    }

    #[test]
    fn parses_decimal_usdc() {
        assert_eq!(parse("0.50").unwrap(), U256::from(500_000));
        assert_eq!(parse("5").unwrap(), U256::from(5_000_000));
    }

    #[test]
    fn rejects_garbage() {
        assert!("abc".parse::<ApprovalAmount>().is_err());
        assert!("1.2345678".parse::<ApprovalAmount>().is_err()); // > 6 decimals
    }
}

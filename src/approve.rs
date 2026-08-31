//! `approve-permit2` — one-time on-chain setup for the `upto` scheme.
//!
//! Grants Uniswap's Permit2 an allowance to move the wallet's USDC, which
//! Permit2-based `upto` settlement requires. This is the ONLY command that
//! broadcasts a transaction (it costs a little Base ETH for gas). The signing
//! key is resolved via the same `SecretResolver`/`op` path as payments and
//! never leaves this process.

use std::io::Write;

use alloy_network::EthereumWallet;
use alloy_primitives::{address, Address, U256};
use alloy_provider::ProviderBuilder;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::sol;
use anyhow::{bail, Context, Result};

use crate::key::{OpCli, SecretResolver};
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

pub async fn run(rpc_url: &str, amount: &str, yes: bool, key_ref: &str) -> Result<()> {
    if key_ref.is_empty() {
        bail!("X402_KEY_REF is not set — needed to resolve the signing key");
    }
    let requested = parse_amount(amount)?;

    // Resolve the key via op (blocking), same path as payments. Never logged.
    let key_ref_owned = key_ref.to_string();
    let key = tokio::task::spawn_blocking(move || OpCli::new().resolve(&key_ref_owned))
        .await
        .context("key resolution task failed")?
        .context("resolving X402_KEY_REF via op")?;
    let signer: PrivateKeySigner = key
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("resolved key is not a valid EVM private key"))?;
    let owner = signer.address();

    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(
            rpc_url
                .parse()
                .with_context(|| format!("invalid --rpc-url '{rpc_url}'"))?,
        );
    let usdc = IERC20::new(BASE_USDC, &provider);

    // Idempotent: skip if already approved for at least the requested amount.
    let current = usdc
        .allowance(owner, PERMIT2)
        .call()
        .await
        .context("reading current Permit2 allowance")?;
    if current >= requested {
        println!(
            "Permit2 is already approved for {owner} (allowance ≥ requested). Nothing to do."
        );
        return Ok(());
    }

    println!("Approve Uniswap Permit2 ({PERMIT2}) to spend USDC from {owner} on Base.");
    if amount.eq_ignore_ascii_case("max") {
        println!("Amount: max (unlimited)");
    } else {
        println!("Amount: {amount} USDC");
    }
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

/// Parse the `--amount` flag: "max" or decimal USDC (e.g. "5" or "0.50").
fn parse_amount(amount: &str) -> Result<U256> {
    if amount.eq_ignore_ascii_case("max") {
        return Ok(U256::MAX);
    }
    let atomic = AtomicUsdc::parse_decimal(amount)
        .map_err(|e| anyhow::anyhow!("invalid --amount '{amount}': {e}"))?;
    Ok(U256::from(atomic.as_atomic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_amount_max_case_insensitive() {
        assert_eq!(parse_amount("max").unwrap(), U256::MAX);
        assert_eq!(parse_amount("MAX").unwrap(), U256::MAX);
    }

    #[test]
    fn parse_amount_decimal_usdc() {
        assert_eq!(parse_amount("0.50").unwrap(), U256::from(500_000));
        assert_eq!(parse_amount("5").unwrap(), U256::from(5_000_000));
    }

    #[test]
    fn parse_amount_rejects_garbage() {
        assert!(parse_amount("abc").is_err());
        assert!(parse_amount("1.2345678").is_err()); // > 6 decimals
    }
}

//! `exact` scheme — EIP-3009 TransferWithAuthorization, USDC on Base.

use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::{address, hex, Address, B256, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol};
use serde_json::{json, Value};

use crate::payment::{AcceptsEntry, Error, PaymentScheme, X402Version};

const BASE_NETWORK: &str = "eip155:8453";
const BASE_CHAIN_ID: u64 = 8453;
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const VALID_AFTER_SLACK_SECS: u64 = 30;

/// Canonical USDC on Base — the ONLY asset we sign for. The spending
/// ceiling assumes this token's 6 decimals; signing for an upstream-chosen
/// asset would let a malicious server bypass the ceiling's economics.
const BASE_USDC: Address = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

/// Upstream-supplied timeouts are clamped: a malicious server must not be
/// able to mint authorizations valid for years (or overflow the math).
const MAX_TIMEOUT_SECS: u64 = 300;

sol! {
    struct TransferWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
}

pub struct ExactEip3009 {
    signer: PrivateKeySigner,
}

impl ExactEip3009 {
    pub fn new(signer: PrivateKeySigner) -> Self {
        Self { signer }
    }

    /// Deterministic core — fixed nonce/timestamps so tests can cross-validate
    /// against the viem-generated vector.
    fn sign_at(
        &self,
        entry: &AcceptsEntry,
        x402_version: X402Version,
        nonce: B256,
        valid_after: u64,
        valid_before: u64,
    ) -> Result<Value, Error> {
        let parse_addr = |s: &str| -> Result<Address, Error> {
            s.parse().map_err(|_| Error::Signing(format!("invalid address '{s}'")))
        };
        let asset = parse_addr(&entry.asset)?;
        let to = parse_addr(&entry.pay_to)?;
        let value: u128 = entry
            .amount
            .parse()
            .map_err(|_| Error::BadAmount(entry.amount.clone()))?;
        let value = U256::from(value);

        let domain = eip712_domain! {
            name: entry.extra.name.clone().unwrap_or_else(|| "USD Coin".into()),
            version: entry.extra.version.clone().unwrap_or_else(|| "2".into()),
            chain_id: BASE_CHAIN_ID,
            verifying_contract: asset,
        };
        let message = TransferWithAuthorization {
            from: self.signer.address(),
            to,
            value,
            validAfter: U256::from(valid_after),
            validBefore: U256::from(valid_before),
            nonce,
        };
        let sig = self
            .signer
            .sign_typed_data_sync(&message, &domain)
            .map_err(|e| Error::Signing(e.to_string()))?;

        Ok(json!({
            "x402Version": x402_version,
            "scheme": "exact",
            "network": BASE_NETWORK,
            "payload": {
                "signature": hex::encode_prefixed(sig.as_bytes()),
                "authorization": {
                    "from": self.signer.address().to_string(),
                    "to": to.to_string(),
                    "value": entry.amount,
                    "validAfter": valid_after.to_string(),
                    "validBefore": valid_before.to_string(),
                    "nonce": format!("{nonce}"),
                },
            },
        }))
    }
}

/// Signer-less support predicate: can the `exact` scheme satisfy this entry?
/// Free function so the proxy can select a payable entry WITHOUT constructing
/// a signer (the key stays untouched until we actually pay). `ExactEip3009`'s
/// trait method delegates here so selection and signing can never diverge.
pub fn supports(entry: &AcceptsEntry) -> bool {
    entry.scheme == "exact"
        && entry.network == BASE_NETWORK
        && entry.asset.parse::<Address>().is_ok_and(|a| a == BASE_USDC)
}

impl PaymentScheme for ExactEip3009 {
    fn supports(&self, entry: &AcceptsEntry) -> bool {
        supports(entry)
    }

    fn sign(&self, entry: &AcceptsEntry, x402_version: X402Version) -> Result<Value, Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before 1970")
            .as_secs();
        let timeout = entry.max_timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECS).min(MAX_TIMEOUT_SECS);
        let nonce = B256::try_random()
            .map_err(|e| Error::Signing(format!("nonce generation failed: {e}")))?;
        self.sign_at(
            entry,
            x402_version,
            nonce,
            now.saturating_sub(VALID_AFTER_SLACK_SECS),
            now + timeout,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payment::{AcceptsEntry, Extra};

    fn test_signer() -> PrivateKeySigner {
        // Throwaway key 0x…01 — publicly known, never funded.
        "0x0000000000000000000000000000000000000000000000000000000000000001"
            .parse()
            .unwrap()
    }

    fn apify_entry() -> AcceptsEntry {
        AcceptsEntry {
            scheme: "exact".into(),
            network: "eip155:8453".into(),
            asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
            amount: "1000000".into(),
            pay_to: "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26".into(),
            max_timeout_seconds: Some(60),
            extra: Extra {
                name: Some("USD Coin".into()),
                version: Some("2".into()),
                facilitator_address: None,
            },
        }
    }

    #[test]
    fn signer_address_matches_known_key() {
        assert_eq!(
            test_signer().address().to_string(),
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"
        );
    }

    #[test]
    fn signature_matches_viem_vector() {
        // Generated by proxies/x402/gen-test-vector.mjs (viem) — byte-identical
        // EIP-712 hashing is the whole point of this test.
        const VIEM_SIG: &str = "0x3404276e9c152bda21d347682f8331e81cefc1b2518b4e28f7ffbb012e720b7d08e5b013085fdc5901092ca2ceb7665696f4ada0af718b3a808450e069ae16451b";
        let scheme = ExactEip3009::new(test_signer());
        let nonce = B256::from(U256::from(1u64));
        let payment = scheme
            .sign_at(&apify_entry(), 2, nonce, 1_700_000_000, 1_700_000_060)
            .unwrap();
        assert_eq!(payment["payload"]["signature"], VIEM_SIG);
    }

    #[test]
    fn payload_shape_matches_x402() {
        let scheme = ExactEip3009::new(test_signer());
        let nonce = B256::from(U256::from(1u64));
        let p = scheme
            .sign_at(&apify_entry(), 2, nonce, 1_700_000_000, 1_700_000_060)
            .unwrap();
        assert_eq!(p["x402Version"], 2);
        assert_eq!(p["scheme"], "exact");
        assert_eq!(p["network"], "eip155:8453");
        let auth = &p["payload"]["authorization"];
        assert_eq!(auth["from"], "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
        assert_eq!(auth["to"], "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26");
        assert_eq!(auth["value"], "1000000");
        assert_eq!(auth["validAfter"], "1700000000");
        assert_eq!(auth["validBefore"], "1700000060");
        assert_eq!(
            auth["nonce"],
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        );
    }

    #[test]
    fn supports_only_exact_on_base() {
        let scheme = ExactEip3009::new(test_signer());
        assert!(scheme.supports(&apify_entry()));
        let mut other = apify_entry();
        other.scheme = "upto".into();
        assert!(!scheme.supports(&other));
        let mut other = apify_entry();
        other.network = "eip155:1".into();
        assert!(!scheme.supports(&other));
    }

    #[test]
    fn rejects_bad_addresses() {
        let scheme = ExactEip3009::new(test_signer());
        let mut bad = apify_entry();
        bad.asset = "not-an-address".into();
        assert!(scheme.sign_at(&bad, 2, B256::ZERO, 0, 1).is_err());
    }

    #[test]
    fn sign_clamps_absurd_timeouts() {
        let scheme = ExactEip3009::new(test_signer());
        let mut entry = apify_entry();
        entry.max_timeout_seconds = Some(u64::MAX); // malicious upstream
        let p = scheme.sign(&entry, 2).unwrap();
        let valid_before: u64 = p["payload"]["authorization"]["validBefore"]
            .as_str().unwrap().parse().unwrap();
        let valid_after: u64 = p["payload"]["authorization"]["validAfter"]
            .as_str().unwrap().parse().unwrap();
        // window is at most slack + clamp, never years
        assert!(valid_before - valid_after <= 30 + 300);
    }

    #[test]
    fn supports_rejects_non_usdc_assets() {
        let scheme = ExactEip3009::new(test_signer());
        let mut evil = apify_entry();
        evil.asset = "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26".into(); // not USDC
        assert!(!scheme.supports(&evil));
        // lowercase canonical USDC must still be accepted
        let mut lower = apify_entry();
        lower.asset = lower.asset.to_lowercase();
        assert!(scheme.supports(&lower));
    }
}

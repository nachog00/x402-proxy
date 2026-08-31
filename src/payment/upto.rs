//! `upto` scheme — Uniswap Permit2 `PermitWitnessTransferFrom`, USDC on Base.
//!
//! Unlike `exact` (EIP-3009), the payer signs an authorization for UP TO a
//! maximum amount; the facilitator later settles the *actual* (lesser) usage
//! on-chain via the x402UptoPermit2Proxy. Two consequences to keep straight:
//!
//! - The signed EIP-712 **domain is Permit2's**, not USDC's — the
//!   `extra.name`/`version` in the 402 response are a red herring for `upto`.
//! - It requires a **one-time on-chain `USDC.approve(Permit2, …)`** from the
//!   payer wallet before any settlement can succeed (see the design spec / the
//!   setup script). `exact` needs no approval.

use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::{address, hex, Address, B256, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol};
use serde_json::{json, Value};

use crate::payment::{
    AcceptsEntry, Error, PaymentScheme, X402Version, BASE_CHAIN_ID, BASE_NETWORK, BASE_USDC,
    MAX_TIMEOUT_SECS, VALID_AFTER_SLACK_SECS,
};

/// Canonical Uniswap Permit2 (same address on every chain). This is the
/// EIP-712 `verifyingContract` for the `upto` signature — NOT the USDC token.
const PERMIT2: Address = address!("000000000022D473030F116dDEE9F6B43aC78BA3");

/// x402UptoPermit2Proxy on Base — the `spender` in the signed permit and the
/// contract that calls Permit2 at settlement. Not carried in the 402 response;
/// a per-network constant (verified on Base mainnet). See design spec risk #1 —
/// confirm this matches the live facilitator before trusting real settlement.
const SPENDER_PROXY: Address = address!("4020A4f3b7b90ccA423B9fabCc0CE57C6C240002");

/// Fallback validity window when the server omits `maxTimeoutSeconds` (upto
/// entries normally set a long one). Still clamped to `MAX_TIMEOUT_SECS`.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

sol! {
    struct TokenPermissions {
        address token;
        uint256 amount;
    }
    struct Witness {
        address to;
        address facilitator;
        uint256 validAfter;
    }
    struct PermitWitnessTransferFrom {
        TokenPermissions permitted;
        address spender;
        uint256 nonce;
        uint256 deadline;
        Witness witness;
    }
}

pub struct UptoPermit2 {
    signer: PrivateKeySigner,
}

impl UptoPermit2 {
    pub fn new(signer: PrivateKeySigner) -> Self {
        Self { signer }
    }

    /// Deterministic core — fixed nonce/timestamps so tests can cross-validate
    /// against a reference Permit2 signer.
    fn sign_at(
        &self,
        entry: &AcceptsEntry,
        x402_version: X402Version,
        nonce: B256,
        deadline: u64,
        valid_after: u64,
    ) -> Result<Value, Error> {
        let parse_addr = |s: &str| -> Result<Address, Error> {
            s.parse()
                .map_err(|_| Error::Signing(format!("invalid address '{s}'")))
        };
        let token = parse_addr(&entry.asset)?;
        let to = parse_addr(&entry.pay_to)?;
        let facilitator = entry
            .extra
            .facilitator_address
            .as_deref()
            .ok_or_else(|| Error::Signing("upto entry missing facilitatorAddress".into()))
            .and_then(parse_addr)?;
        let amount: u128 = entry
            .amount
            .parse()
            .map_err(|_| Error::BadAmount(entry.amount.clone()))?;

        let domain = eip712_domain! {
            name: "Permit2",
            chain_id: BASE_CHAIN_ID,
            verifying_contract: PERMIT2,
        };
        let message = PermitWitnessTransferFrom {
            permitted: TokenPermissions {
                token,
                amount: U256::from(amount),
            },
            spender: SPENDER_PROXY,
            nonce: U256::from_be_bytes(nonce.0),
            deadline: U256::from(deadline),
            witness: Witness {
                to,
                facilitator,
                validAfter: U256::from(valid_after),
            },
        };
        let sig = self
            .signer
            .sign_typed_data_sync(&message, &domain)
            .map_err(|e| Error::Signing(e.to_string()))?;

        Ok(json!({
            "x402Version": x402_version,
            "scheme": "upto",
            "network": BASE_NETWORK,
            "payload": {
                "signature": hex::encode_prefixed(sig.as_bytes()),
                "permit2Authorization": {
                    "permitted": { "token": token.to_string(), "amount": entry.amount },
                    "from": self.signer.address().to_string(),
                    "spender": SPENDER_PROXY.to_string(),
                    "nonce": format!("{nonce}"),
                    "deadline": deadline.to_string(),
                    "witness": {
                        "to": to.to_string(),
                        "facilitator": facilitator.to_string(),
                        "validAfter": valid_after.to_string(),
                    },
                },
            },
        }))
    }
}

/// Signer-less support predicate: can `upto` satisfy this entry? Free function
/// so the proxy selects a payable entry WITHOUT constructing a signer. Requires
/// the facilitator address, which must be bound into the witness.
pub fn supports(entry: &AcceptsEntry) -> bool {
    entry.scheme == "upto"
        && entry.network == BASE_NETWORK
        && entry.asset.parse::<Address>().is_ok_and(|a| a == BASE_USDC)
        && entry.extra.facilitator_address.is_some()
}

impl PaymentScheme for UptoPermit2 {
    fn supports(&self, entry: &AcceptsEntry) -> bool {
        supports(entry)
    }

    fn sign(&self, entry: &AcceptsEntry, x402_version: X402Version) -> Result<Value, Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before 1970")
            .as_secs();
        let timeout = entry
            .max_timeout_seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);
        let nonce = B256::try_random()
            .map_err(|e| Error::Signing(format!("nonce generation failed: {e}")))?;
        self.sign_at(
            entry,
            x402_version,
            nonce,
            now + timeout,
            now.saturating_sub(VALID_AFTER_SLACK_SECS),
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

    fn apify_upto_entry() -> AcceptsEntry {
        AcceptsEntry {
            scheme: "upto".into(),
            network: "eip155:8453".into(),
            asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
            amount: "1000000".into(),
            pay_to: "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26".into(),
            max_timeout_seconds: Some(18000),
            extra: Extra {
                name: Some("USD Coin".into()),
                version: Some("2".into()),
                facilitator_address: Some("0x14fDa13953Fc30428938E6BF950d036e77214e52".into()),
            },
        }
    }

    #[test]
    fn supports_upto_on_base_usdc_with_facilitator() {
        assert!(supports(&apify_upto_entry()));

        let mut e = apify_upto_entry();
        e.scheme = "exact".into();
        assert!(!supports(&e), "wrong scheme");

        let mut e = apify_upto_entry();
        e.network = "eip155:1".into();
        assert!(!supports(&e), "wrong network");

        let mut e = apify_upto_entry();
        e.asset = "0x0000000000000000000000000000000000000001".into();
        assert!(!supports(&e), "non-USDC asset");

        let mut e = apify_upto_entry();
        e.extra.facilitator_address = None;
        assert!(!supports(&e), "missing facilitator");
    }

    #[test]
    fn payload_shape_matches_permit2() {
        let scheme = UptoPermit2::new(test_signer());
        let nonce = B256::from(U256::from(1u64));
        // sign_at(entry, ver, nonce, deadline, valid_after)
        let p = scheme
            .sign_at(&apify_upto_entry(), 2, nonce, 1_700_000_060, 1_700_000_000)
            .unwrap();
        assert_eq!(p["x402Version"], 2);
        assert_eq!(p["scheme"], "upto");
        assert_eq!(p["network"], "eip155:8453");

        let auth = &p["payload"]["permit2Authorization"];
        // Addresses compared by value, not checksum casing.
        let addr = |v: &Value| v.as_str().unwrap().parse::<Address>().unwrap();
        assert_eq!(addr(&auth["permitted"]["token"]), BASE_USDC);
        assert_eq!(auth["permitted"]["amount"], "1000000");
        assert_eq!(addr(&auth["from"]), test_signer().address());
        assert_eq!(addr(&auth["spender"]), SPENDER_PROXY);
        assert_eq!(auth["deadline"], "1700000060");
        assert_eq!(
            addr(&auth["witness"]["to"]),
            "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26"
                .parse::<Address>()
                .unwrap()
        );
        assert_eq!(
            addr(&auth["witness"]["facilitator"]),
            "0x14fDa13953Fc30428938E6BF950d036e77214e52"
                .parse::<Address>()
                .unwrap()
        );
        assert_eq!(auth["witness"]["validAfter"], "1700000000");
        assert_eq!(
            auth["nonce"],
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        );
    }

    #[test]
    fn signs_deterministically_and_wellformed() {
        let scheme = UptoPermit2::new(test_signer());
        let nonce = B256::from(U256::from(1u64));
        let a = scheme
            .sign_at(&apify_upto_entry(), 2, nonce, 1_700_000_060, 1_700_000_000)
            .unwrap();
        let b = scheme
            .sign_at(&apify_upto_entry(), 2, nonce, 1_700_000_060, 1_700_000_000)
            .unwrap();
        assert_eq!(a, b, "same inputs must produce the same signature");
        let sig = a["payload"]["signature"].as_str().unwrap();
        assert!(sig.starts_with("0x") && sig.len() == 132, "65-byte hex sig");
    }

    #[test]
    fn rejects_bad_addresses() {
        let scheme = UptoPermit2::new(test_signer());
        let mut bad = apify_upto_entry();
        bad.pay_to = "not-an-address".into();
        assert!(scheme
            .sign_at(&bad, 2, B256::ZERO, 1, 0)
            .is_err());
    }
}

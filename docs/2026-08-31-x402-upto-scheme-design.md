# x402 `upto` scheme (Permit2) — design spec

**Date:** 2026-08-31
**Status:** implemented on `feat/x402-upto` (signer viem-cross-validated; prefers `upto` over `exact`). Pending: the one-time Permit2 approval + confirming `SPENDER_PROXY` against Apify's live facilitator before real settlement (risk #1).
**Builds on:** `2026-07-16-x402-proxy-rust-design.md` (the `exact` scheme + `PaymentScheme` seam)

## Goal

Add `upto` as a second `PaymentScheme` so paid calls settle the **actual metered
usage** (cents) instead of `exact`'s flat maximum. Against Apify this turns a
**$1.00 flat** `call-actor` into roughly its true cost. Prefer `upto` when the
server offers it; fall back to `exact`.

## What `upto` actually is (authoritative — Coinbase x402 spec + verified contracts)

Not EIP-3009. It signs a **Uniswap Permit2 `PermitWitnessTransferFrom`** with an
x402-specific witness. Source of truth: `coinbase/x402`
`specs/schemes/upto/scheme_upto_evm.md` + the Base-mainnet-verified
`x402UptoPermit2Proxy` contract + Uniswap `permit2`.

**EIP-712 domain is Permit2's, NOT USDC's** (the single biggest gotcha — the
`extra.name:"USD Coin"/version:"2"` in the 402 response is a red herring for
`upto`; it only applies to `exact`/EIP-3009):

```
name: "Permit2"          # keccak256("Permit2")
chainId: 8453
verifyingContract: 0x000000000022D473030F116dDEE9F6B43aC78BA3   # canonical Permit2
# NO version field
```

**Type (EIP-712), copy-paste:**
```
PermitWitnessTransferFrom(TokenPermissions permitted,address spender,uint256 nonce,uint256 deadline,Witness witness)TokenPermissions(address token,uint256 amount)Witness(address to,address facilitator,uint256 validAfter)
```

**Three distinct addresses — do not conflate:**
| Field | Value (Base) | Role |
|---|---|---|
| `verifyingContract` | `0x0000…78BA3` | Permit2 (signing domain only) |
| `spender` | `0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002` | `x402UptoPermit2Proxy` — the settling contract; **NOT in the 402 response**, a per-network constant |
| `witness.facilitator` | `extra.facilitatorAddress` (`0x14fDa…`) | off-chain settler EOA bound into the sig |

**Message** (`amount` = the MAX / ceiling):
```
permitted = { token: USDC, amount: <max, e.g. 1000000> }
spender   = 0x4020A4f3…                 # proxy constant
nonce     = <unordered 256-bit, single-use>
deadline  = now + maxTimeoutSeconds
witness   = { to: payTo, facilitator: extra.facilitatorAddress, validAfter: now - slack }
```

**`_meta["x402/payment"]` payload** (diffs vs `exact` marked):
```json
{ "x402Version":2, "scheme":"upto", "network":"eip155:8453",
  "payload": {
    "signature": "0x…",
    "permit2Authorization": {                      // exact uses "authorization"
      "permitted": { "token":"0x…USDC", "amount":"1000000" },
      "from":     "0x…payer",
      "spender":  "0x4020A4f3…",
      "nonce":    "0x…32-byte…",
      "deadline": "<unix>",                         // plays exact's validBefore role
      "witness":  { "to":"0x…payTo", "facilitator":"0x…", "validAfter":"<unix>" }
    } } }
```

**Settlement ("up to"):** client signs the max; server computes actual usage
(≤ max); facilitator EOA calls the proxy, which enforces
`msg.sender == witness.facilitator` and `requested ≤ permitted.amount`, then pulls
`requested` USDC from payer → `payTo`.

## Prerequisite — a real UX cliff

Permit2 requires a **one-time on-chain `USDC.approve(Permit2 0x0000…78BA3, max)`**
from the payer wallet. Without it, every `upto` settlement reverts. `exact` needs
zero approval — this is the cost of the cheaper scheme.

- The proxy **cannot** and must not do this (house rule: never touch the key). We
  ship a **helper script the user runs** (approve once, `type(uint256).max`).
- Ordering: `exact` stays the safe default until the wallet is approved; once
  approved, `upto` is preferred for cost.

## Architecture changes

- `payment.rs` `Extra`: add `facilitator_address: Option<String>` (currently
  silently dropped).
- `payment/upto.rs`: `UptoPermit2` `PaymentScheme` impl + `upto::supports` free fn.
  `supports` = `scheme=="upto" && network==Base && asset==USDC &&
  extra.facilitator_address.is_some()`.
- `SPENDER_PROXY` constant `0x4020A4f3…` (see risk below).
- Proxy selection: iterate schemes in preference order `[upto, exact]`, take the
  first `supports()` entry that also passes the ceiling. Single lazy signer
  (`OnceCell`) shared by both schemes.
- `AmountGuard`: applies to `permitted.amount` (the max) — existing logic, no
  change; conservative (you can never be charged more than the ceiling).

## Verification plan (same rigor as `exact`)

- **Signature cross-validation:** oracle = **`x402-chain-eip155`** Rust crate
  (`v2_eip155_upto`) as a dev-dependency — no Node needed. Fixed throwaway key
  `0x…01`, fixed nonce/deadline/validAfter; assert our 65-byte sig matches.
- **Unit:** `supports`/selection precedence, payload shape, the vector.
- **Integration (bridge.rs):** mock upstream offering `upto`; assert the proxy
  signs Permit2 and retries with `permit2Authorization`.
- **Real Apify:** after the user's one-time approval, one real `call-actor`
  settles the ACTUAL cents; confirm the on-chain draw is < the $1 max.

## Risks / open items

1. **`SPENDER_PROXY = 0x4020A4f3…` is a hardcoded constant, not in the 402
   response.** Confirmed deployed+verified on Base mainnet, but NOT yet confirmed
   that Apify's mainnet facilitator routes through this exact proxy. Wrong spender
   → settlement rejected. **Confirm before any real spend** (cross-check against
   `x402-chain-eip155`'s network constant table + a dry run).
2. **Permit2 approval prerequisite** (above) — hard blocker until done.
3. Repo may have moved to `x402-foundation/x402` (same paths) — mirror check only.

## Out of scope

- Networks other than Base; assets other than USDC.
- Automating the Permit2 approval (user-run, key stays with user).

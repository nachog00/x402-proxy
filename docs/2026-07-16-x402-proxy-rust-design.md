# x402-proxy — Rust x402 MCP signing proxy

**Date:** 2026-07-16
**Task:** godchat mcps#12 (x402-proxy-rust)
**Status:** approved
**Replaces:** Node prototype in `proxies/x402/` (kept as reference until parity)

## Goal

A single static Rust binary, `x402-proxy`, that lets Claude Code use paid HTTP
MCP servers (first target: `https://mcp.apify.com?payment=x402`) by
transparently signing x402 USDC payments. Replaces the Node prototype with:

- no Node/npm dependency or supply-chain surface in the key-touching path
- key resolved from 1Password lazily, in-memory only, never on disk
- integration into the `mcps` catalog/install flow
- a reference implementation of x402 + EIP-712 signing in Rust

## Architecture

Convert the repo to a Cargo workspace:

```
Cargo.toml              # [workspace] members = ["crates/mcps", "crates/x402-proxy"]
crates/mcps/            # existing CLI, moved as-is
crates/x402-proxy/
  src/main.rs           # clap CLI; startup: upstream connect → stdio serve
  src/proxy.rs          # rmcp stdio server ↔ rmcp streamable-http client bridge
  src/payment.rs        # x402 types (PaymentRequired, accepts[]), scheme selection
  src/payment/exact.rs  # EIP-3009 TransferWithAuthorization signing via alloy
  src/key.rs            # SecretResolver trait, op CLI backend, lazy Zeroizing cache
```

Module layout follows house style: `foo.rs + foo/`, no `mod.rs`.

**Dependencies (x402-proxy):** `rmcp` (server + client; stdio and
streamable-http-client transports), `alloy-signer-local` + `alloy-sol-types` +
`alloy-primitives` (EIP-712), `tokio`, `clap` (derive), `serde`/`serde_json`,
`thiserror` (per-module errors), `anyhow` (main), `zeroize`, `rand`.

**Boundary:** the proxy knows nothing about catalogs or `secrets.toml`. Its
whole interface is `--upstream <url>` plus env `X402_KEY_REF` (an `op://` ref)
and optional `X402_MAX_AMOUNT`.

## Data flow

1. **Startup:** connect to upstream via streamable HTTP, `tools/list`, cache
   the tool list. No key access at startup.
2. **Serve:** expose cached tools over stdio MCP. `tools/list` → cached list;
   `tools/call` → forward upstream verbatim.
3. **Payment interception:** if a `tools/call` result is an error whose text
   parses as JSON containing an `accepts` array, select a supported
   `(scheme, network)` entry, sign, retry once with the payment payload in
   `_meta["x402/payment"]`. A second failure is returned to the client as-is.
4. **v1 simplification (explicit):** tools only. No resources, prompts,
   sampling, or notification passthrough (upstream target is tools-only).

## Payment module

- `PaymentScheme` trait:
  `supports(&AcceptsEntry) -> bool`,
  `sign(&AcceptsEntry, signer) -> Result<PaymentPayload>`.
- v1 ships one implementation, `ExactEip3009`, for `exact` on `eip155:8453`
  (USDC on Base): EIP-712 `TransferWithAuthorization` declared via alloy
  `sol!`; domain name/version from `extra` (defaults `"USD Coin"` / `"2"`);
  random 32-byte nonce; `validAfter = now − 30s`;
  `validBefore = now + maxTimeoutSeconds` (default 60).
- Payload shape matches the Node prototype / x402 v2:
  `{ x402Version, scheme, network, payload: { signature, authorization } }`.
- **Spending guard:** `X402_MAX_AMOUNT` (decimal USDC, e.g. `0.50`) is
  **required** — the proxy refuses to sign any payment when unset or when the
  requested amount exceeds it, returning a clear tool error naming the amount
  and the ceiling. `accepts[].amount` arrives in atomic units; convert using
  the asset's decimals (6 for USDC) before comparing. Auto-signing money must have an explicit ceiling.
- `upto` (Permit2) and other networks are out of scope; the trait is the
  extension point.

## Key handling

- `SecretResolver` trait in `key.rs`:
  `resolve(&self, secret_ref: &str) -> Result<Zeroizing<String>>`.
  v1 backend: `OpCli`, spawning `op read <ref>` (first-party, signed, same
  pattern as `mcps secrets.rs`). A future backend may wrap the 1Password SDK
  (no official Rust SDK exists yet; the community `onepassword` crate wraps
  the official core — rejected for v1 to keep unofficial code out of the
  key path).
- **Lazy, first-use resolution:** no key access at startup. The first
  payment-required response triggers `resolve()`; the derived alloy
  `PrivateKeySigner` is cached in a `tokio::sync::OnceCell` for the process
  lifetime. One `op` prompt per session at most; free calls never touch the
  key.
- Consequences by design:
  - Claude Code launching its MCP servers never fires a 1Password prompt.
  - The 1Password unlock prompt fires exactly when money is about to move —
    a human consent gate layered under `X402_MAX_AMOUNT`.
  - If `op` fails (locked, missing), only the paid call errors with a clear
    message; the proxy stays up.
- Key material: held in `Zeroizing`, never logged, never in argv, never in
  Claude config (config carries only the `op://` ref). Stderr logs amount,
  asset, and recipient before signing — never key material.

## mcps integration

Catalog grows an optional proxy table (serde `deny_unknown_fields`):

```toml
[server.proxy]
kind = "x402"                  # only kind for now
key = "secret:x402-wallet"     # uid → op:// ref via secrets.toml
max_amount = "0.50"            # USDC ceiling per payment
```

When present, `mcps install` registers the server as **stdio** instead of
http:

```
claude mcp add <name> \
  -e X402_KEY_REF=<op://ref> -e X402_MAX_AMOUNT=<ceiling> \
  -- x402-proxy --upstream <url>
```

The uid resolves to the **ref**, not the secret value (refs over content).
`mcps` checks `x402-proxy` is on PATH (installed via
`cargo install --path crates/x402-proxy`) and errors helpfully if not.

No container for v1 — it is a locally built, trusted static binary, not npx.
A Containerfile can follow if isolation is wanted later.

## Errors

Per-module `thiserror` enums (`proxy`, `payment`, `key`), `anyhow` at `main`.
Payment failures surface to the MCP client as tool errors with actionable
context (scheme mismatch, over-ceiling, op failure, upstream retry failure).

## Testing

- **Unit:** payment-required extraction (error / non-error / malformed JSON /
  missing `accepts`), scheme selection, amount guard (unset, under, over),
  `SecretResolver` behind a mock.
- **Signature cross-validation:** a test vector generated by the Node
  prototype (throwaway key, fixed nonce and timestamps) checked into the
  tests; the Rust signer must produce the identical signature.
- **Integration:** in-process rmcp mock upstream returning a
  payment-required error; assert the proxy signs, retries, and succeeds, and
  that over-ceiling requests are refused without signing.

## Out of scope (v1)

- `upto` / Permit2 scheme, non-Base networks
- resources / prompts / notifications passthrough
- 1Password SDK backend (trait extension point exists)
- container packaging for the proxy
- cross-process key agent (per-session in-process cache is sufficient)

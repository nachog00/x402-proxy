# x402-proxy

[![CI](https://github.com/nachog00/x402-proxy/actions/workflows/ci.yml/badge.svg)](https://github.com/nachog00/x402-proxy/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/x402-proxy.svg)](https://crates.io/crates/x402-proxy)

A single static Rust binary that lets an MCP client use **paid HTTP MCP servers**
with no account — it transparently signs [x402](https://x402.org) USDC
micropayments on Base per call.

It runs as a **stdio MCP server** in front of your client (e.g. Claude Code) and
bridges to an upstream **streamable-HTTP MCP server**. When the upstream returns
an x402 payment-required error, the proxy signs a USDC payment and retries once,
injecting the payload into `_meta["x402/payment"]`.

## Schemes

Both are USDC-on-Base, EIP-712 via [alloy](https://github.com/alloy-rs/alloy),
each cross-validated **byte-for-byte** against a `viem` reference vector
(`vectors/gen-*-vector.mjs`):

- **`exact`** — EIP-3009 `TransferWithAuthorization`. Fixed amount; no on-chain setup.
- **`upto`** — Uniswap **Permit2** `PermitWitnessTransferFrom`. Authorizes *up to*
  a max; the facilitator settles the actual metered usage (often far cheaper).
  Requires a one-time `USDC.approve(Permit2)` — see `approve-permit2` below.

Selection prefers `upto`; the spend ceiling applies to its max.

## Install

```sh
cargo install --path crates/x402-proxy
```

## Usage

```sh
# Run the proxy (what an MCP host launches):
X402_KEY_REF='op://<vault>/<item>/<field>' X402_MAX_AMOUNT='0.50' \
  x402-proxy serve --upstream 'https://mcp.apify.com?payment=x402'

# One-time Permit2 setup (required before `upto` can settle; costs a little gas):
X402_KEY_REF='op://<vault>/<item>/<field>' \
  x402-proxy approve-permit2            # --rpc-url, --amount max|<usdc>, --yes
```

- `X402_KEY_REF` — an `op://` reference; the key is resolved lazily via 1Password
  (`op read`) on the first payment only, held in `Zeroizing`, never on disk/argv/logs.
- `X402_MAX_AMOUNT` — required decimal-USDC ceiling; the proxy refuses to sign
  when unset or when a demand exceeds it.

## Key handling

The signing key never leaves the process: config carries only the `op://` ref,
resolution is lazy and off the hot path, the value lives in `Zeroizing`, and it
is never logged or placed in argv. Launching your MCP servers never prompts
1Password — the unlock fires exactly when money moves.

## Regenerating the signature vectors

The Rust signers are asserted equal to `viem` output. To regenerate (container
only — no Node on the host):

```sh
podman run --rm -v ./vectors:/app:Z -w /app docker.io/library/node:24-slim \
  sh -c "npm config set ignore-scripts true && npm install --omit=dev && node gen-upto-vector.mjs"
```

## History

Extracted from the [`mcps`](../mcps) monorepo, which references this as a PATH
binary via its catalog `[server.proxy]` table. Design specs are in `docs/`.

## Roadmap

Tracked follow-ups (carried over from the `mcps` monorepo at extraction):

- **`x402-rs` signer adoption (deferred).** `x402-rs` ships a client-side EVM
  Permit2 `upto` signer too (`V2Eip155UptoClient`); its proxy/spender address is
  byte-identical to ours, independently confirming our `SPENDER_PROXY` constant.
  Keeping our viem-validated `exact`+`upto` signers for now; revisit calling their
  free fns (`sign_erc3009_authorization`, `sign_permit2_upto_authorization`,
  `x402-chain-eip155` feature `client`) behind our `PaymentScheme` trait only if
  maintenance becomes a burden. Friction: their output is a base64 V2 envelope
  we'd reshape into our leaner `_meta` JSON, and their signers have no unit tests.
- **Upstream contribution.** Donate our viem cross-validation vectors / signing
  unit tests to `x402-rs` — their EVM signers have zero unit tests and no
  reference vectors; exactly our strength.
- **Richer `_meta` envelope.** Evaluate the fuller `{accepted, resource, …}`
  payload from the x402 MCP spec for conformance, and Permit2 EIP-2612
  gas-sponsoring (skip the permit when allowance/nonce already suffice).
- **JSON-RPC error interception.** Today we intercept payment-required only when it
  arrives as an `is_error` tool result; also handle it arriving as a JSON-RPC error.
- **More networks/schemes.** Non-Base EVM networks and additional schemes are
  `PaymentScheme`/network extension points.
- **Deployment.** A nix flake and/or a prebuilt static-musl binary (a container is
  awkward given the `op` key-access requirement).
- **End-to-end.** Observe a *successful* actor producing a small positive metered
  `upto` settlement against real Apify (their actor infra was flaky during
  testing; a failed actor correctly settled $0).

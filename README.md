# x402-proxy

[![CI](https://github.com/nachog00/x402-proxy/actions/workflows/ci.yml/badge.svg)](https://github.com/nachog00/x402-proxy/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/x402-proxy.svg)](https://crates.io/crates/x402-proxy)
[![codecov](https://codecov.io/gh/nachog00/x402-proxy/graph/badge.svg)](https://codecov.io/gh/nachog00/x402-proxy)

Use **paid MCP servers without an account.** `x402-proxy` is a small stdio MCP
server you place in front of a paid HTTP MCP server; when a tool call requires
payment, it signs an [x402](https://x402.org) USDC micropayment on Base and
retries the call — transparently, per call, up to a ceiling you set. Your signing
key stays in 1Password and is only ever touched the moment money actually moves.

First target: Apify's MCP (`https://mcp.apify.com?payment=x402`) — no Apify token
required.

## Install

```sh
cargo install x402-proxy
```

## Quickstart

Register it with your MCP client (here, the Claude Code CLI) as a stdio server
that fronts a paid upstream:

```sh
claude mcp add apify \
  --transport stdio \
  -e X402_KEY_REF='op://Private/x402-wallet/private-key' \
  -e X402_MAX_AMOUNT='0.50' \
  -- x402-proxy serve --upstream 'https://mcp.apify.com?payment=x402'
```

That's it. The upstream's tools now appear in your client: free calls pass
through untouched, and a paid call is signed automatically (up to your ceiling)
and retried — no API token, no account.

Two environment variables control it:

- **`X402_KEY_REF`** — a 1Password `op://` reference to your funded EVM private
  key. It's resolved lazily (only when a payment is actually due), and never
  written to disk, argv, or logs. Starting your MCP servers never prompts
  1Password — the unlock happens exactly when a payment is signed.
- **`X402_MAX_AMOUNT`** — a required per-payment ceiling in USDC. The proxy
  refuses to sign if it's unset, or if a demand exceeds it.

### One-time setup for the cheaper `upto` scheme

`upto` settles through Uniswap Permit2, which needs a single on-chain approval
from your wallet first (a few cents of Base ETH for gas):

```sh
X402_KEY_REF='op://Private/x402-wallet/private-key' x402-proxy approve-permit2
```

## Payment schemes

Both are USDC on Base, EIP-712, and cross-validated byte-for-byte against a
`viem` reference implementation:

- **`exact`** — EIP-3009. Pays a fixed amount; no on-chain setup.
- **`upto`** — Permit2. Authorizes *up to* a maximum; the facilitator then
  settles the actual (often much smaller) metered cost. Needs the approval above.

When a server offers both, the proxy prefers `upto`; your ceiling always applies
to the authorized maximum.

## How it works

A payment-required tool error advertises the accepted `(scheme, network, amount)`
options. The proxy picks a supported one, signs the payment, and retries the call
once with the signed payload in `_meta["x402/payment"]`. Only two things ever see
your key — 1Password (to read it) and this binary (to sign) — and it's held in
memory only, never persisted.

## Documentation

- Design specs: [`docs/`](docs/)
- Building, tests, signature vectors, releases, and roadmap:
  [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md)

## License

MIT © nachog00

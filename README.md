# x402-proxy

[![CI](https://github.com/nachog00/x402-proxy/actions/workflows/ci.yml/badge.svg)](https://github.com/nachog00/x402-proxy/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/x402-proxy.svg)](https://crates.io/crates/x402-proxy)
[![codecov](https://codecov.io/gh/nachog00/x402-proxy/graph/badge.svg)](https://codecov.io/gh/nachog00/x402-proxy)

Use **paid MCP servers without an account.** `x402-proxy` is a small stdio MCP
server you place in front of a paid HTTP MCP server; when a tool call requires
payment, it signs an [x402](https://x402.org) USDC micropayment on Base and
retries the call — transparently, per call, up to a ceiling you set. Your signing
key can live in 1Password (or an env var, a file, or the config) and is only ever
touched the moment money actually moves.

First target: Apify's MCP (`https://mcp.apify.com?payment=x402`) — no Apify token
required.

## Install

```sh
cargo install x402-proxy
```

## Quickstart

Register the proxy in front of a paid upstream. `install` prints portable
`mcpServers` JSON by default — paste it into any client — or pass `--client
claude` to run `claude mcp add` for you:

```sh
# Print JSON for any MCP client (the default)
x402-proxy install --upstream https://mcp.apify.com \
  --key-ref env:APIFY_X402_KEY --max 0.50

# …or register it with Claude directly
x402-proxy install --client claude --upstream https://mcp.apify.com \
  --key-ref op://Private/x402-wallet/private-key --max 0.50
```

A bare `--upstream` host gets `?payment=x402` appended automatically. For Claude,
`--scope local|project|user` is forwarded to `claude mcp add` (default `local`).

That's it. The upstream's tools now appear in your client: free calls pass
through untouched, and a paid call is signed automatically (up to your ceiling)
and retried — no API token, no account.

Two environment variables (which `install` writes for you) control it:

- **`X402_KEY_REF`** — where the funded EVM private key comes from. It's resolved
  lazily (only when a payment is actually due) and never written to disk, argv,
  or logs. Supported sources:
  - `op://Vault/item/field` — 1Password (unlock happens exactly when a payment
    is signed, never at startup)
  - `env:VAR_NAME` — read from an environment variable
  - `file:/path/to/key` — read from a file
  - `wallet:NAME` — a named wallet from the config file (see below)
  - `0x…` — a raw inline key (fine for a throwaway / low-stakes wallet; prefer
    one of the above for real funds)
- **`X402_MAX_AMOUNT`** — a required per-payment ceiling in USDC. The proxy
  refuses to sign if it's unset, or if a demand exceeds it.

### Config file (named wallets)

Define your wallets once in `~/.config/x402-proxy/config.toml` (override with
`$X402_PROXY_CONFIG`) and reference them by name — most setups want a single
wallet for all x402 funding:

```toml
default_wallet = "main"

[wallets.main]
key = "op://Private/x402-wallet/private-key"   # any key source above
max = "0.50"                                    # default ceiling for this wallet

[wallets.dev]
key = "env:X402_DEV_KEY"
```

Then a wallet is all you need — its `max` becomes the default ceiling:

```sh
x402-proxy install --upstream https://mcp.apify.com --wallet main
# or, since `default_wallet = "main"`, just:
x402-proxy install --upstream https://mcp.apify.com
```

### One-time setup for the cheaper `upto` scheme

`upto` settles through Uniswap Permit2, which needs a single on-chain approval
from your wallet first (a few cents of Base ETH for gas):

```sh
# Any key source works here too — op://, env:, file:, wallet:, or raw 0x…
X402_KEY_REF='wallet:main' x402-proxy approve-permit2
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
your key — its source (1Password, env, or file) and this binary (to sign) — and
it's held in memory only, never persisted.

## Documentation

- Design specs: [`docs/`](docs/)
- Building, tests, signature vectors, releases, and roadmap:
  [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md)

## License

MIT © nachog00

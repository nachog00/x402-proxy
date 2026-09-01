# Changelog

Managed by [knope](https://knope.tech) — add entries with `knope document-change`.

## 0.1.2 (2026-09-01)

### Fixes

- CI: bump-in-PR release model so releases never push to protected main
- Refactor the CLI into per-command modules (internal; no behavior change)

## 0.1.0

### Features

- Initial release: a static Rust stdio MCP proxy that auto-signs x402 USDC
  micropayments on Base to reach paid MCP servers, with:
  - `exact` (EIP-3009) and `upto` (Uniswap Permit2) payment schemes, each
    cross-validated byte-for-byte against a `viem` reference vector;
  - lazy 1Password (`op read`) key resolution held in `Zeroizing`, never on
    disk/argv/logs;
  - a required per-payment spend ceiling and asset/network allow-listing;
  - an `approve-permit2` command for the one-time Permit2 setup.

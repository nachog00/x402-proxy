# Changelog

Managed by [knope](https://knope.tech) — add entries with `knope document-change`.

## 0.1.5 (2026-09-01)

### Fixes

#### Prebuilt binaries via `cargo binstall`

Each release now attaches prebuilt binaries for Linux (`x86_64`, `aarch64`),
macOS (`x86_64`, `aarch64`), and Windows (`x86_64`), so `cargo binstall
x402-proxy` installs without compiling. Falls back to a source build on other
targets.

## 0.1.4 (2026-09-01)

### Features

- install command + multi-source key resolution (#10)

#### Add `install` command and multi-source key resolution

`x402-proxy install` registers the proxy in front of an upstream MCP server —
printing portable `mcpServers` JSON by default, or running `claude mcp add` with
`--client claude` (and `--scope local|project|user`). A bare `--upstream` host
gets `?payment=x402` appended automatically.

`X402_KEY_REF` now resolves from any of `op://…` (1Password), `env:VAR`,
`file:/path`, `wallet:NAME`, or a raw `0x…` key — not just 1Password. Named
wallets live in `~/.config/x402-proxy/config.toml` (a key source plus a default
ceiling), referenced by `--wallet NAME` or `X402_KEY_REF=wallet:NAME`. The
`approve-permit2` setup honors the same sources.

## 0.1.3 (2026-09-01)

### Fixes

- CI: publish with `--no-verify` (CI already builds and tests the exact commit)

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

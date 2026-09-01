---
default: minor
---

# Add `install` command and multi-source key resolution

`x402-proxy install` registers the proxy in front of an upstream MCP server —
printing portable `mcpServers` JSON by default, or running `claude mcp add` with
`--client claude` (and `--scope local|project|user`). A bare `--upstream` host
gets `?payment=x402` appended automatically.

`X402_KEY_REF` now resolves from any of `op://…` (1Password), `env:VAR`,
`file:/path`, `wallet:NAME`, or a raw `0x…` key — not just 1Password. Named
wallets live in `~/.config/x402-proxy/config.toml` (a key source plus a default
ceiling), referenced by `--wallet NAME` or `X402_KEY_REF=wallet:NAME`. The
`approve-permit2` setup honors the same sources.

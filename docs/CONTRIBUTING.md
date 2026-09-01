# Contributing

## Build & test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Coverage

```sh
cargo tarpaulin --workspace --engine llvm --out Stdout
```

~77% line coverage. The payment/signing core (`payment.rs`, `exact.rs`,
`upto.rs`, `proxy.rs`) is 80–98% covered; the gaps are the live-I/O edges that
need a real chain or server (`main.rs`'s `serve` wiring and `approve.rs`'s
on-chain `run()`), which are verified manually instead.

## Signature vectors

The Rust signers are asserted equal, byte-for-byte, to `viem` output. Vectors
live in `vectors/`. To regenerate (container only — no Node on the host):

```sh
podman run --rm -v ./vectors:/app:Z -w /app docker.io/library/node:24-slim \
  sh -c "npm config set ignore-scripts true && npm install --omit=dev && node gen-upto-vector.mjs"
```

Paste the printed signature into the matching `signature_matches_viem_vector` test.

## Releases

Automated with [knope](https://knope.tech) + changesets. The rule: nothing is
ever pushed to `main` directly — a release always arrives via a PR.

1. In a PR **into `dev`**, run `knope document-change` (or `cargo make
   changeset`) and commit the `.changeset/` file it creates. CI requires it.
2. Merging to `dev` triggers a bot that prepares the release (version bump +
   changelog + consumed changesets) on a `release-next` branch and opens a
   **"Release vX.Y.Z"** PR into `main`.
3. Review and merge that PR. A post-merge job tags `vX.Y.Z`, creates the GitHub
   release, publishes to crates.io, and fast-forwards `dev`. Nothing pushes to
   `main` outside the PR merge.

Pre-1.0 semantics: a `minor` change bumps the patch; a breaking change bumps the
minor.

## Roadmap

- **`x402-rs` signer adoption (deferred).** `x402-rs` ships a client-side EVM
  Permit2 `upto` signer too (`V2Eip155UptoClient`); its proxy/spender address is
  byte-identical to ours, independently confirming our `SPENDER_PROXY` constant.
  We keep our viem-validated `exact`+`upto` signers for now; revisit calling
  their free fns (`sign_erc3009_authorization`, `sign_permit2_upto_authorization`
  in `x402-chain-eip155`, feature `client`) behind our `PaymentScheme` trait only
  if maintenance becomes a burden. Friction: their output is a base64 V2 envelope
  we'd reshape into our leaner `_meta` JSON, and their signers have no unit tests.
- **Upstream contribution.** Donate our viem cross-validation vectors / signing
  unit tests to `x402-rs` — their EVM signers have no reference vectors.
- **Richer `_meta` envelope.** Evaluate the fuller `{accepted, resource, …}`
  payload from the x402 MCP spec, and Permit2 EIP-2612 gas-sponsoring.
- **JSON-RPC error interception.** Also handle payment-required arriving as a
  JSON-RPC error, not only as an `is_error` tool result.
- **More networks/schemes** — non-Base EVM networks and further schemes are
  `PaymentScheme`/network extension points.
- **Deployment.** A nix flake and/or a prebuilt static-musl binary.

## History

Extracted (with full history, via `git subtree`) from a personal `mcps`
monorepo, which references this as a PATH binary in its catalog `[server.proxy]`
table.

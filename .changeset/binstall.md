---
default: patch
---

# Prebuilt binaries via `cargo binstall`

Each release now attaches prebuilt binaries for Linux (`x86_64`, `aarch64`),
macOS (`x86_64`, `aarch64`), and Windows (`x86_64`), so `cargo binstall
x402-proxy` installs without compiling. Falls back to a source build on other
targets.

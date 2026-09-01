---
default: patch
---

# Cache release builds (CI only)

The per-target release binary jobs now use `Swatinem/rust-cache`, so re-releases
reuse compiled dependencies instead of building every crate cold. No user-facing
change.

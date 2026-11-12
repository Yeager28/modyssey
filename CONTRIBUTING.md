# Contributing to Modyssey

## The one hard rule

**This repository must remain an independent implementation.**

This codebase derives from Modyssey (MIT, © SulfurNitride), which is itself an
independent Rust implementation of the documented `.wabbajack` modlist
format. Its MIT license — and this project's freedom to be distributed under
any terms the owner chooses — depends on that independence.

Therefore:

1. **Never paste, port, translate, or reference source code from:**
   - `wabbajack-tools/wabbajack` (GPLv3) — the C# original;
   - `modde-rs` or any other GPL-licensed modlist installer;
   - any local Wabbajack fork/checkout.
2. **Allowed references:** public format and behavior *documentation* — the
   Wabbajack wiki, this repo's spec notes, and black-box testing of released
   `.wabbajack` files. File formats and functional behavior are not
   copyrightable; copied code is.
3. **If you need to know how Wabbajack handles something:** restate the
   behavior from the docs or from observed output, write it fresh in Rust,
   and note the source in the PR description. Never open Wabbajack source
   "just to check".
4. **If a PR introduces code that looks ported** (struct/field naming that
   mirrors C# internals, comments quoting Wabbajack source), maintainers will
   reject it and ask for a clean-room rewrite.

## Practical notes

- Keep the MIT LICENSE and ATTRIBUTION.md intact in all distributions.
- Upstream git history must never be rewritten or squashed away — the
  lineage is part of the compliance story.
- Windows is the primary target; keep `#[cfg(unix)]` gating intact so Linux
  builds stay working where practical.

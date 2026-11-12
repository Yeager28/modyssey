# Contributing to Modyssey

Modyssey is a free, open-source, multi-game modlist installer — a Wabbajack
alternative built in Rust. Contributions are welcome.

## Contribution terms (GPL-3.0-or-later)

- The project is licensed **GPL-3.0-or-later** ([LICENSE](LICENSE)). By
  submitting a contribution — code, documentation, or anything else intended
  for the repository — you agree it is licensed under those same terms and
  that you have the right to license it so. You keep your copyright; the git
  history keeps your name.
- The engine seed is MIT-licensed CLF3 code
  ([LICENSE-MIT-CLF3](LICENSE-MIT-CLF3), [ATTRIBUTION.md](ATTRIBUTION.md)).
  MIT code may flow *into* this project; GPL code does not flow *out* of it.
- Do not submit code you cannot license to the project under GPL-3.0-or-later
  (e.g. code you don't own, or proprietary-licensed code).
- No private builds: every distributed binary corresponds to a public commit
  (GPL source-publication discipline).

## Wabbajack reference policy

**Reading and porting Wabbajack C# source
([wabbajack-tools/wabbajack](https://github.com/wabbajack-tools/wabbajack),
GPLv3) is permitted in this project.** Modyssey is GPL-3.0-or-later, so
derivatives of Wabbajack's code remain GPL-compliant. When you port a
behavior from Wabbajack, leave a provenance comment naming the source file
(e.g. `// ported from Wabbajack.Compiler/ArchiveChooser.cs`) so the lineage
stays traceable. What you write under that policy is GPL, like the rest of
the project.

## Hard rule — Nexus free-tier downloads

**Never automate, script, bot-drive, or scrape Nexus Mods free-tier
downloads.** The free-user flow is exactly two things:

1. Modyssey opens the mod's download page in the **user's own browser**
   (a single "open this URL" action), and
2. Modyssey **watches a local folder** for files the user downloads
   themselves, hash-verifying them on arrival.

No headless browsers, no download-link extraction, no retry loops against
nexusmods.com, no workarounds for the site's bot protection. The Premium
path uses the official documented API only, exactly as MO2/Vortex/Wabbajack
do. Automating the free tier violates the Nexus ToS, gets API keys blocked,
and poisons this project's standing in the modding ecosystem — such PRs
will be rejected.

## Practical notes

- Windows is the primary target; keep `#[cfg(unix)]`/`#[cfg(windows)]`
  gating intact so Linux keeps building.
- `cargo check --all-targets` before you push; keep `cargo fmt` and
  `cargo clippy` clean.
- Keep the engine (library code) UI-free: GUI and CLI are consumers of the
  engine, not parts of it.
- Upstream CLF3 git history must never be rewritten or squashed away.

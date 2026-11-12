# Attribution

This project, **Modyssey**, is a rebranded fork/derivative of
**CLF3** by **SulfurNitride** — <https://github.com/SulfurNitride/CLF3> —
licensed under the MIT License (see [LICENSE](LICENSE)).

- **Upstream:** CLF3 commit `a6f332a65d112ee22fb3abb9700e6e4f8512a32d`
  ("Release CLF3 0.2.9 with MOMW installs, Steam login and a game file index",
  2026-11-06).
- CLF3's `Cargo.toml` declares `license = "MIT"`. The upstream repository does
  not ship a standalone LICENSE file, so this repository carries the standard
  MIT text with the upstream copyright line preserved (above), satisfying the
  license's only condition.
- The git history of this repository preserves the full upstream commit
  history; the rebrand commit is grafted on top of upstream `HEAD` without
  rewriting or squashing anything.

## What was changed from upstream

- Project identity: package/binary/lib renamed `modyssey` → `modyssey`; window
  titles, CLI strings, user agents, data directories, keyring service name
  and log filenames rebranded to "Modyssey".
- Windows portability: Linux-only code paths (flock-based install locks,
  `set_permissions(..., 0o755)`, signal handlers, symlink spill) are gated
  behind `#[cfg(unix)]` with Windows fallbacks (pid-file locks, plain copies).
- Game registry: added a Dark Souls II: Scholar of the First Sin entry
  (Steam app ID 335300) so its modlists auto-detect the game directory.

## Hard rule — independent implementation

CLF3 (and therefore this fork) is an **independent Rust implementation** of
the documented `.wabbajack` file format. Its MIT status depends on that
independence. **Never paste, port, or transliterate Wabbajack C# source code
into this repository** — behavioral/format documentation only. The same rule
applies to any other GPL-licensed codebase (e.g. modde-rs). See
[CONTRIBUTING.md](CONTRIBUTING.md).

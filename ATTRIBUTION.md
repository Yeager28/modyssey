# Attribution

Modyssey is a **GPL-3.0-or-later** project. This file records where the code
comes from and which license terms travel with each lineage.

## CLF3 — the engine seed (MIT)

The installer engine at the heart of Modyssey is a fork of **CLF3** by
**SulfurNitride** — <https://github.com/SulfurNitride/CLF3> — licensed under
the MIT License.

- **Upstream base:** CLF3 commit `a6f332a65d112ee22fb3abb9700e6e4f8512a32d`
  ("Release CLF3 0.2.9 with MOMW installs, Steam login and a game file
  index", 2026-11-06).
- This repository preserves the complete upstream commit history (132
  commits). Our work is grafted on top of upstream `HEAD` without rewriting
  or squashing anything — the lineage is part of the compliance story.
- MIT is GPL-compatible: the CLF3-derived code is incorporated into this
  GPL-3.0-or-later project, and the verbatim MIT text with SulfurNitride's
  copyright line is preserved forever in
  [LICENSE-MIT-CLF3](LICENSE-MIT-CLF3).
- Per-file provenance: files substantially unchanged from CLF3 keep their
  MIT header comments; new and heavily modified code is GPL-3.0-or-later.
- **Changes made on top of upstream:** project identity (Modyssey), a
  Windows port (`#[cfg(unix)]`/`#[cfg(windows)]` gates, pid-file install
  locks), and a Dark Souls II: Scholar of the First Sin game entry
  (Steam app ID 335300).

## Wabbajack — the reference implementation (GPLv3)

**Wabbajack** by the wabbajack-tools organization —
<https://github.com/wabbajack-tools/wabbajack> — is the incumbent modlist
installer Modyssey is building an alternative to, and the reference
implementation whose behavior Modyssey ports (most importantly the modlist
compiler, planned for milestone M5).

- **Policy:** reading and porting Wabbajack's C# source is **permitted and
  acknowledged** in this project, precisely *because* Modyssey is
  GPL-3.0-or-later: a derivative of a GPL work stays GPL, which is the
  honest, compliant arrangement. Modules ported from Wabbajack carry their
  upstream reference (source file and behavior) in code comments.
- The `.wabbajack` container/manifest format itself is publicly documented;
  the CLF3-derived engine was an independent implementation of that format
  and remains MIT-lineage code.
- "Wabbajack" is used here nominatively, to identify the incumbent for
  comparison and interop. No affiliation or endorsement is implied; no
  Wabbajack assets or wordmarks are used.

## License map

| Thing | License | Where |
|---|---|---|
| Modyssey as a whole | GPL-3.0-or-later | [LICENSE](LICENSE) |
| CLF3-derived engine code | MIT (© SulfurNitride) | [LICENSE-MIT-CLF3](LICENSE-MIT-CLF3) |
| Modlist data (`.modyssey` lists) | policy notice, not code license | [MODLIST-POLICY.md](MODLIST-POLICY.md) |

See also [ACKNOWLEDGMENTS.md](ACKNOWLEDGMENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md).

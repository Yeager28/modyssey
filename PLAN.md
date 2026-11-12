# PLAN — "Majula" (working name): an open-source, Rust + GPUI modlist installer

**Version:** 1.0 · **Date:** 2026-11-13 · **Status:** PLAN ONLY — awaiting owner approval. Nothing in this document has been implemented; it is the deliverable.

**One-line pitch:** A free, open-source, multi-game modlist installer and compiler — a modern alternative to Wabbajack — built in Rust with a GPUI (Zed's UI framework) front-end, a first-class no-Premium-required Nexus flow, its own `.modfile` container format (while reading `.wabbajack` lists today), and Dark Souls II: Scholar of the First Sin as the flagship supported game.

---

## 0. Document scope & sources

This plan is grounded in three on-disk assets plus targeted online research (all URLs in Appendix A):

1. **The engine seed** — `H:/Games/ds2-modpack-app/`: a rebranded fork of CLF3 (github.com/SulfurNitride/CLF3, MIT; Rust, egui, `.wabbajack` installer) with a working Windows port and a DS2 game entry. State per `VERDICT.md` (2026-11-12, four-gate assessment): release build **PASS** (8 m 22 s, `ds2modpack.exe` 50.4 MB), 318 tests (290 pass / 9 Windows-env fails / 19 ignored — none in the installer core), all four needed directive handlers present, MIT attribution in order, ~70 files of rebrand/port work **uncommitted**.
2. **The reference implementation** — `H:/Games/wabbajack-fork/`: Wabbajack C# v4.3.2.2 + the 18-line `DarkSoulsII` game patch (branch `dark-souls-2-support`), ~40-project .NET solution (Compiler, Installer, NexusApi, DTOs, WabbajackClientApi, App.Avalonia, …), GPLv3.
3. **The research corpus** — `H:/Games/DS2 Modlist/.frostfx-research/wabbajack-plan.md` (deep WJ mechanics analysis with wiki/code citations) and `wj-phase3b-private-recompile.md` (our compiled list's anatomy).
4. **The acceptance target** — `H:/Games/wabbajack-fork/output/DS2-Modpack-by-Bonkfyre-1.0.1.wabbajack` (42,358,346 B, SHA-256 `65E0F05A25F19AC4A4291B3FE16B95C3052BF8D677C7113D4499C59554E3BD0B`): 15 archives (11 Nexus / 2 HTTP / 2 manual), 10,616 directives (10,505 `FromArchive` + 79 `PatchedFromArchive` + 30 `InlineFile` + 2 `RemappedInlineFile`), 7.98 GB downloads, 12.8 GB installed, `GameType: "DarkSoulsII"`.

Design reference: the owner's Modrinth screenshot (`local://pasted-image-bdb262d96eda0982.png`, analyzed 2026-11-13) + https://modrinth.com/discover/mods (brand accent `#1bd96a` verified live via the site's `meta-theme-color`).

---

## 1. Executive summary & positioning

**What we're building.** A desktop app for Windows (Linux later) that installs *modlists* — complete, reproducible modded game setups — from a single small file, without redistributing anyone's mods. It downloads each required mod from its original source (Nexus, GitHub, direct HTTP, or the user's own browser session), verifies every byte by xxHash64, applies the list's install directives (copy-from-archive, binary patch, inline file, path-remapped config), and produces a working modded game instance. It also compiles modlists: point it at a source Mod Organizer 2 instance and it emits a `.modfile` containing only hashes, source pointers, patches and configs — never mod bytes.

**Positioning.** "The open-source Wabbajack alternative." Wabbajack (wabbajack-tools/wabbajack, GPLv3, C#/Avalonia) is the incumbent: mature, trusted, gallery + CDN infrastructure, ~50 supported games. We are not a hostile fork — we are a fresh Rust/GPUI successor that:

- **installs existing `.wabbajack` modlists unchanged** (the format is documented and the CLF3-derived engine already parses/installs our showcase list),
- **introduces `.modfile`**, a cleaner v1 container with explicit format versioning and a modlist-data license field,
- **treats the free Nexus user as a first-class citizen, not an afterthought**: Wabbajack's answer to a free account is an exception thrown mid-queue (`NexusDownloader.cs` L122–124: *"Nexus Premium is required for automatic downloads"*); our answer is a designed queue UX — per-archive browser-open, Downloads-folder pickup, hash verification, advance/retry — plus the Premium API as an opt-in accelerator,
- **is multi-game by data, not by code**: games live in a shipped `games.json` registry (DS2 SOTFS first-class, Steam app 335300); adding a game is a data edit, not a recompile — an explicit improvement over both Wabbajack's compile-time `GameRegistry.cs` and the prototype's compile-time `KNOWN_GAMES`,
- **looks like a 2026 product**: GPUI (GPU-accelerated, hybrid immediate/retained mode; the framework Zed ships on Windows stable today) with a Modrinth-inspired dark design system — rounded cards, one vibrant green accent, clean typography.

**Why Rust.** The engine seed is already Rust and already works on Windows (see VERDICT gates a–d). Rust gives memory safety during multi-GB streaming installs, cheap parallelism (rayon/tokio already in the prototype), single-binary distribution with no .NET runtime prerequisite, and a supply chain we can audit. The rewrite cost of the UI (egui → GPUI) is the price of the owner's GPUI mandate and is confined to the shell — the engine is kept, not rewritten.

**Non-goals for v1.** No mod hosting/CDN of our own (we consume sources, never mirror); no DRM, no license enforcement code (see §3 — policy is documentation, not code); no Linux parity in v1 UI (engine stays cross-platform); no MO2-replacement features (we *produce* MO2 instances, exactly like Wabbajack).

---

## 2. Name & branding

Three candidates, checked against crates.io and GitHub for collisions (2026-11-13):

| | Name | Story | Collision check | Verdict |
|---|---|---|---|---|
| ★ | **Majula** | Dark Souls II's hub village — the warm place every path returns to, where the fire is kept. A perfect identity for the owner's flagship game, instantly recognizable to the Souls community, yet a plain proper noun that works for a multi-game tool. Reads well lowercase (`majula`), short, pronounceable. | **crates.io: free** (404 on lookup). GitHub: 55 repos named *-majula\**, none in the mod-manager/installer space (top hits: MajulaRecomp, a DS2 Switch recomp project — adjacent fandom, different product; rs-majula, a RuneScape server; a Go message queue). No npm/PyPI presence of consequence. | **Recommended** |
| 2 | **Bonfire** | The universal Souls motif ("rest at the bonfire"), and it harmonizes beautifully with the author brand: *"Bonfire, by Bonkfyre."* Warm, community-coded, easy to say. | crates.io: name occupied (a yanked 0.0.0 "Client library for the Bonfire API"). Generic word — heavy general-purpose collision surface (Bonfire API/bot ecosystem, campfire-style apps) makes searchability mediocre. | Runner-up; usable if we accept `bonfire-app` as the crate name |
| 3 | **Caravan** | Elder Scrolls Khajiit caravans — goods carried across the land, stop by stop. A lovely metaphor for the per-archive manual-download flow (each caravan stop = one browser visit), and it keeps Wabbajack's Tamrielic naming spirit. | crates.io: occupied (dormant 2020 personal package manager, 78 LOC). Generic word, large collision surface (Mozilla Caravan history, routers, travel apps). | Fallback |

**Rejected after checking:** *Modwright* (an npm MCP server for game modding is actively publishing under that exact name, 2026-10 — in-domain collision); *Netch* (17.7k-star C# proxy client owns the name); *Estus* (crates.io squatted: "Reserved name for future use").

**Recommendation: Majula.** Distinctive, empty in our ecosystem corners, deeply on-brand for the DS2-first story, and short enough to type in a terminal. Brand line: **"Majula — modlists, kept warm. A Wabbajack alternative, by Bonkfyre."** The "by Bonkfyre" author credit rides along in the README, About screen, and release notes — the product keeps its own identity so it can outlive any one list or author.

Branding notes:

- Repo: `github.com/himanshudbg/majula` (owner's account); crates published as `majula`, `majula-core`, etc.
- Icon direction: a stylized DS2 "rotating majula bonfire" mark — circular, ember gradient on the dark surface color; avoid FromSoftware asset copies (draw our own).
- Trademark caution (details §3): "Majula" is a place name from Dark Souls II; fan-tool use of a lore proper noun is long-standing community practice, but the README should carry a short disclaimer ("Not affiliated with or endorsed by FromSoftware or Bandai Namco") and we must never ship game assets in the repo or the app binary.
- If the owner prefers Bonfire or wants a non-FromSoftware name entirely, nothing in this plan depends on the string; swap at M0 with zero architectural impact.

---

## 3. Licensing & policy

### 3.1 The app: GPL-3.0 (required by our method, and chosen)

- **Chosen:** the whole project is **GPL-3.0-or-later**. This is also the *correct* license given the declared method: the compiler (M5) and several engine behaviors will be **ported from Wabbajack's GPLv3 C# source** ("using Wabbajack's C# code as reference" is the owner's stated intent), which makes those modules derivatives of a GPL work; the honest, compliant move is to be GPL-3.0 end-to-end from day one.
- **CLF3 lineage preserved:** the engine seed is MIT (SulfurNitride, pinned upstream commit `a6f332a65d112ee22fb3abb9700e6e4f8512a32d`). MIT is GPL-compatible: we incorporate it, keep `LICENSE-MIT-CLF3` (the full MIT text with SulfurNitride's copyright line) and `ATTRIBUTION.md` in the repo root forever, and license our modifications + new code GPL-3.0. Per-file provenance: files substantially unchanged from CLF3 keep their MIT header comment; everything else gets the GPL header.
- **Policy change from the prototype:** CLF3's README hard rule — "never paste Wabbajack C# source" — existed because the fork was MIT. Under GPL-3.0 that rule is *retired and inverted*: reading/porting Wabbajack C# is permitted and acknowledged, provided the derivative stays GPL-3.0. `CONTRIBUTING.md` and `ATTRIBUTION.md` must be rewritten accordingly (this is a day-1 M0 task so no contributor is confused).
- **Credit:** `ACKNOWLEDGMENTS.md` credits (1) SulfurNitride / CLF3 (MIT), (2) wabbajack-tools / Wabbajack (GPLv3) as the reference implementation whose behavior we port, (3) the octodiff binary-patch format authors.

### 3.2 What GPL does and does not do (the owner's "no selling" goal, stated accurately)

GPL-3.0 **does not prohibit commercial use** of the app. Anyone may sell copies or offer paid support; what GPL *requires* is that recipients get the source and the same freedoms (§4: "may not restrict"). The owner's intent — "so that anyone cannot use it to sell modlists or modify it for commercial purposes" — has two parts:

1. *"Modify it for commercial purposes"* → **covered**: closed-source commercial derivatives are prohibited by GPL. ✔
2. *"Sell modlists"* → **not governed by the code license at all** — modlist *data* is not the program. ✔ addressed by policy instead:

### 3.3 Modlist-data policy (the Wabbajack-style dual model, as documentation — not code enforcement)

- The `.modfile` manifest carries a `License` field (§4). **Default: `CC BY-NC-SA 4.0`** — non-commercial, share-alike for the list's data (its structure, configs, patch blobs as a collection). Authors may declare any other license for their own lists.
- We ship a **`MODLIST-POLICY.md` template** (the Wabbajack precedent: their `LICENSE.txt` declares lists CC BY-NC-SA and prohibits paywalling, enforced socially/technically by their team, not by the license of the code). Our template states: lists built with Majula are free to share; access must never be payment-gated; donations are fine — exactly the Wabbajack FAQ position ("The installer must be free, donations must be a 'thank you' — not a purchase").
- The app **displays** the declared license on the list's detail screen and in the install wizard. It does **not** refuse to install anything — the tool stays dumb; the norms live in the documents and the community. This is deliberate: code-level enforcement of data licensing is both technically futile (trivially stripped) and hostile to the modding ecosystem we want to join.

### 3.4 Trademark & fair use rules (hard rules for all marketing)

- "Wabbajack" may be used **nominatively**: "an alternative to Wabbajack", "installs Wabbajack (.wabbajack) modlists" — identifying the incumbent for comparison is classic fair use. Never: their logo, their wordmark in our branding, any phrasing implying endorsement or affiliation, or a confusingly similar name.
- "Modrinth": *aesthetic inspiration only*. We borrow the design *language* (dark surfaces, rounded cards, a vibrant green accent, clean grotesque typography) — never their logo, layout copy, or product name. Modrinth's brand green is `#1bd96a`; our token sheet uses it as the starting accent with the explicit note that a final brand pass at M0 may shift the hue a few degrees for distinct identity (colors per se are not protectable, but brand distinctiveness is good manners and good marketing).
- "Majula": lore proper noun; carry a one-line non-affiliation disclaimer for FromSoftware/Bandai Namco in README and About.
- "Nexus", "Nexus Mods": refer to the service factually ("requires a free Nexus Mods account"); never imply partnership.

---

## 4. `.modfile` format spec v1

### 4.1 Design constraints

1. **Frictionless `.wabbajack` interop** — a WJ manifest must parse as a subset of ours with zero transformation. WJ's manifest is JSON with **PascalCase keys**, `GameType` as a plain string, `Archives[]` with downloader `State` objects tagged by `$type` (`"NexusDownloader, Wabbajack.Lib"`, `"HttpDownloader, Wabbajack.Lib"`, `"ManualDownloader, Wabbajack.Lib"`), and `Directives[]` tagged by `$type` (`FromArchive`, `PatchedFromArchive`, `InlineFile`, `RemappedInlineFile`, …). The prototype's `modlist/types.rs` already deserializes all of this strictly (`#[serde(rename_all = "PascalCase")]`, `#[serde(tag = "$type")]`) — that model **is** the interop layer and is kept verbatim as the v1 schema core.
2. **Lossless `.wabbajack` → `.modfile` conversion** must be byte-faithful: same manifest semantics, same blobs, install output identical (M2 acceptance).
3. **Explicit versioning** — WJ has no format version field (compat is by `WabbajackVersion` heuristics); we add `FormatVersion`.
4. **Author-visible policy** — the `License` field (§3.3).

### 4.2 Container

- **File:** `*.modfile` — a standard **ZIP** archive (ZIP64, so blob entries > 4 GiB are legal).
  - Why ZIP over WJ's 7z: first-class pure-Rust support in the prototype's existing `zip` crate; ubiquitous tooling for debugging; Unicode-safe entry names if we follow the spec (forward slashes only — WJ's zip extractor had a whole `BackslashEntryTests` suite for this bug; we normalize entry paths on read).
- **Magic marker (format sniffing):** a zero-priority entry at the archive root named **`modfile`** (mirroring WJ's `modlist` entry naming) whose first bytes are the JSON manifest? No — keep it simpler and faster to sniff: root entry **`MODFILE.v1.marker`** containing `{"format":"modfile","version":1}`. Detection order for any input file: extension `.modfile` → open zip → read marker; extension `.wabbajack` (or 7z signature) → open 7z → read `modlist` entry → WJ-import path. Unknown container → clear error naming what we found.
- **Entries:**
  - `MODFILE.v1.marker` — magic + format version
  - `manifest.json` — the manifest (§4.3)
  - `<uuid4>` — binary blobs (inline files, patch deltas, remapped-inline bodies), named by UUID exactly like WJ so conversion is a straight copy of entries. Blob identity is established by the directives that reference them (WJ semantics); we additionally record each blob's xxHash64 in the manifest's `BlobIndex` (new, optional) for corruption detection before use.
- **Compression:** Deflate for manifest/marker; blobs are stored **as-is (Store)** — patch deltas and inline blobs are already compressed or binary; recompressing buys little and costs CPU on 100 MB+ blobs (our showcase list's largest blob is a 16.3 MB octodiff delta; inline `nvngx_dlssnr.dll`-scale files arrive as patches, never raw).

### 4.3 Manifest schema (v1)

JSON, UTF-8, **PascalCase keys** (WJ compatibility). Fields new relative to WJ are marked **[new]**; everything else matches WJ's `ModList` DTO shape as already implemented in `modlist/types.rs`:

```jsonc
{
  "FormatVersion": 1,                      // [new] int, required. Gates directive-set evolution.
  "Name": "DS2 Modpack by Bonkfyre",
  "Version": "1.0.1",                      // .NET-style version string (WJ convention)
  "Author": "Bonkfyre",
  "Description": "…",
  "GameType": "DarkSoulsII",               // plain string, resolved via games registry (never an enum — WJ's own
                                            // string-enum converter means unknown values are data, and stock WJ
                                            // hard-fails them; we resolve via registry and warn if unknown)
  "ToolVersion": "majula 0.3.0",           // [renamed from WabbajackVersion] tool that built the file
  "Readme": "https://…",
  "Website": "https://…",
  "License": "CC BY-NC-SA 4.0",            // [new] modlist-data license notice (§3.3); default when absent:
                                            // "CC BY-NC-SA 4.0" (WJ parity); empty = "Undeclared"
  "BlobIndex": [                           // [new][optional] entry-name → {Size, Hash} for early corruption checks
    { "Entry": "209c5341-…", "Size": 16339463, "Hash": "…" }
  ],
  "Archives": [                            // WJ Archive shape, unchanged
    {
      "Name": "Elden Ring HUD 1.3-1430.7z",
      "Hash": "wTS1EfZkRxU=",              // xxHash64, base64 (WJ encoding)
      "Size": 173000000,
      "State": { "$type": "NexusDownloader, Wabbajack.Lib",
                 "GameName": "darksouls2", "ModID": 1430, "FileID": 5339 }
    }
    // HttpDownloader {Url}, ManualDownloader {Url, Prompt} keep WJ field names
  ],
  "Directives": [                          // WJ Directive shape, unchanged tag semantics
    { "$type": "FromArchive", "To": "mods\\…", "Hash": "…", "Size": …,
      "FromHash": "…", "ArchiveHash": "…", "FromPath": "…" },
    { "$type": "PatchedFromArchive", …, "PatchID": "209c5341-…" },
    { "$type": "InlineFile", "To": "…", "Hash": "…", "Size": …, "DataID": "<uuid>" },
    { "$type": "RemappedInlineFile", "To": "…", "Hash": "…", "Size": …, "DataID": "<uuid>",
      "SourceDataID": "<uuid>" }
    // TransformedTexture, CreateBSA: supported (prototype handlers exist), unused by the DS2 list
  ]
}
```

### 4.4 Directive set & extension point

- **v1 implements the WJ core four** (`FromArchive`, `PatchedFromArchive`, `InlineFile`, `RemappedInlineFile`) — exactly the set our showcase list uses (10,616 directives) — **plus** `TransformedTexture` and `CreateBSA` (both already implemented in the prototype's texture/BSA pipelines; needed for Bethesda-list parity later).
- **Unknown `$type` = hard fail, fail-fast** (WJ behavior; the prototype already aborts before any download on unknown directives — VERDICT gate (c)). Versioning is by `FormatVersion`: a v1 consumer refuses a v2 file with "this list needs Majula ≥ x.y", never half-installs.
- **Forward-compatible advisory extension:** directive types prefixed `x-` (e.g. `x-majula-ui-hint`) are **advisory metadata** — an engine that doesn't recognize an `x-` directive *ignores it with a warning* instead of failing. This is the sanctioned extension point for future features (UI hints, post-install steps, telemetry opt-outs) without burning format versions. Core (non-`x-`) directives are frozen within a `FormatVersion`.
- **Path remapping:** `RemappedInlineFile` keeps WJ's magic-token scheme (`{--||GAME_PATH_MAGIC_BACK||--}` etc.) so config files compiled by Wabbajack remap correctly under our installer, and vice versa where possible.

### 4.5 `.wabbajack` import path (M2) and the future compiler (M5)

- **Import (install-time, no conversion needed):** the engine's manifest model already parses a WJ `modlist` entry directly; the importer opens the 7z, parses, and installs — identical code path, different container reader. This is what makes Majula a *drop-in* Wabbajack alternative on day one.
- **Conversion (explicit, lossless):** `majula convert in.wabbajack -o out.modfile` reads manifest + blobs, re-emits the zip container with marker + `manifest.json` (WJ fields verbatim, `FormatVersion: 1`, `License` defaulted, `ToolVersion` noting the conversion), blob entries copied byte-for-byte. Acceptance (M2): installing `out.modfile` produces a tree whose per-file xxHash64 set equals the `.wabbajack` install's.
- **Compiler (M5):** emits `.modfile` natively from a source MO2 instance — the Rust port of `Wabbajack.Compiler` (§5.4). Compiled lists default `License: CC BY-NC-SA 4.0`, author-editable in the compile wizard.

---

## 5. Architecture

### 5.1 Workspace layout (Cargo workspace)

```
majula/                          # github.com/himanshudbg/majula (GPL-3.0-or-later)
├── Cargo.toml                   # [workspace]
├── crates/
│   ├── majula-core/             # engine: hashing, manifest model, .modfile/.wabbajack readers,
│   │                            #   verification, install pipeline, patching (octodiff), BSA/textures,
│   │                            #   archive extraction (zip/7z/rar), progress event stream
│   ├── majula-sources/          # downloaders: nexus (premium API + OAuth), nexus-manual (free flow),
│   │                            #   http, mega, google-drive, mediafire, loverslab, wabbajack-cdn;
│   │                            #   download dispatcher + rate limiter; Downloads-folder watcher
│   ├── majula-games/            # data-driven game registry: embedded default games.json +
│   │                            #   user overlays; game finders (Steam/GOG/registry/Heroic)
│   ├── majula-compiler/         # (M5) MO2 instance → .modfile; NoMatch reporting; allow-lists
│   ├── majula-app/              # GPUI shell (the UI; design-system module inside, Zed `ui`-crate pattern)
│   └── majula-cli/              # clap CLI: install, info, convert, compile, games, hash-file
├── data/games.json              # the shipped registry (also embedded via include_str!)
├── LICENSE                      # GPL-3.0
├── LICENSE-MIT-CLF3             # preserved MIT text (SulfurNitride)
├── ATTRIBUTION.md / ACKNOWLEDGMENTS.md / MODLIST-POLICY.md / SECURITY.md
└── .github/workflows/ci.yml     # Windows + Linux: build, test, clippy, fmt, cargo-audit
```

`majula-compiler` is stubbed as an empty crate at M0 (so the workspace shape is final) and filled at M5 — no skeleton code ships in between.

### 5.2 Key design decisions

- **Engine/UI separation is absolute.** `majula-core`/`majula-sources` are pure libraries with **zero GPUI dependencies**; they expose progress and state as an event stream (`tokio` channels / callbacks). The CLI and the GPUI app are two thin consumers. This is the pattern that let the prototype grow an egui GUI and a CLI over one engine, and it de-risks the GPUI bet: if GPUI stalls, the CLI still ships lists (and the GUI can be re-shelled without touching the engine).
- **Hashing is WJ-compatible xxHash64** (base64, `"XXXXXXXXXXXXXXX="`), already implemented in `hash.rs` and round-trip verified against the WJ fork's `hash-file` output during the phase-3b work.
- **Patching is octodiff** (`octodiff.rs`, `DeltaReader`) — reads the exact patch blobs inside existing `.wabbajack` files; the compiler (M5) emits the same format (reference: WJ's octodiff fork and `PatchCache/`).
- **Async model:** engine runs on tokio (prototype status quo); the GPUI app drives it via GPUI's background executor + entity updates (§6.4) — one tokio runtime owned by the app, engine spawned as tasks, progress drained on the UI thread. The prototype's sidecar/progress-JSON machinery (`progress_json.rs`, `sidecar.rs`) is the seam: it already decouples engine progress from any particular frontend.
- **Secrets:** port `secrets.rs` to the `keyring` crate with a **Windows Credential Manager** backend (the prototype only has freedesktop Secret Service — on Windows it degrades to plaintext; hardening gap flagged in VERDICT). Nexus API keys/OAuth tokens go to the OS credential store, never to plain settings files.

### 5.3 Migration table — prototype module → disposition (with WJ C# reference for gaps)

| Prototype module (`src/`) | Disposition | Notes / WJ reference for gaps |
|---|---|---|
| `modlist/{types,mod,db,browser,update,install_manifest}.rs` | **KEEP** → `majula-core` | The serde PascalCase/`$type` model *is* the WJ interop layer; keep field-for-field. Ref: `Wabbajack.DTOs/ModList.cs`, `DTOs/DownloadStates/*` |
| `installer/{processor,pipeline,streaming,host,prevalidation,progress*,sidecar,bsa_reuse}.rs`, `installer/handlers/{from_archive,patched,inline,…}` | **KEEP** → `majula-core` | All four core handlers verified against our list (VERDICT gate c). Gaps to port from `Wabbajack.Installer`: `InstallPathRules.cs` (refuse game-folder/special-folder installs), `FileDeletionRules.cs` (upgrade-over-existing semantics) |
| `octodiff.rs` | **KEEP** → `majula-core` | Reads/writes WJ patch blobs |
| `hash.rs` | **KEEP** → `majula-core` | Ref: `Wabbajack.Hashing.xxHash64` |
| `archive/sevenzip.rs` (+ zip/unrar deps) | **KEEP** → `majula-core::archive` | Ref: `Wabbajack.FileExtractor` (signature-based extractor dispatch) |
| `bsa/`, `textures/` (+ wgpu/bc7 stack) | **KEEP** → `majula-core` | For Bethesda parity; unused by DS2 list. Ref: `Wabbajack.Compression.BSA`, `Wabbajack.Hashing.PHash` |
| `downloaders/{nexus,http,mega_native,mega_transfer,google_drive,mediafire,loverslab,wabbajack_cdn,yandex,nexus_files}.rs` | **KEEP** → `majula-sources` | Add: **free-user manual lane** as a first-class downloader state (§7). Ref: `Wabbajack.Downloaders.Nexus` (`NexusDownloader.cs`), `Downloaders.ManualSources`, `Downloaders.Dispatcher`, `RateLimiter` |
| `game_finder/{known_games,steam,heroic,proton,vdf}.rs` | **PORT** → `majula-games` (data-driven) | `known_games.rs` compile-time `KNOWN_GAMES` → `data/games.json` + overlay; finders (Steam vdf, Heroic, registry) port as-is. Ref: `Wabbajack.DTOs/Game/GameRegistry.cs` (metadata shape), `Downloaders.GameFile/GameLocator.cs` |
| `game_index/` | **KEEP** (defer wiring) | Per-game file-hash index for preflight hints; no DS2 section needed (our list has zero `GameFileSource`) |
| `secrets.rs` | **REWRITE** | Windows Credential Manager via `keyring` crate (see §5.2) |
| `settings.rs`, `paths.rs` | **KEEP** → `majula-core` | Windows-appdata layout (`%LOCALAPPDATA%/majula`) |
| `browser_gui.rs`, `momw_gui.rs`, `collections_gui*.rs` (egui) | **REWRITE** in GPUI → `majula-app` | 100% UI redesign (owner requirement); no egui code carries over |
| `momw/`, `ttw/`, `collection/`, `collection_app/`, `steam/`, `fluorine.rs` | **DEFER** | Keep compiling behind cargo features, exclude from v1 UI — scope control (they're CLF3's OpenMW/Tale-of-Two-Wastelands/collections tooling; revisit post-1.0) |
| `main.rs` (clap CLI) | **PORT** → `majula-cli` | Verbs: `install`, `info`, `convert`, `compile` (M5), `games`, `hash-file`, `browser` (GUI launcher) |
| — *(missing)* **compiler** | **NEW** → `majula-compiler` (M5) | Ref: `Wabbajack.Compiler/` — `MO2Compiler.cs` (roots: MO2 + downloads + game; meta-only archives), `ACompiler.cs` (MakeStack matching cascade), `CompilationSteps/*` (DirectMatch, IncludeTaggedFiles, IncludePatches, IncludeStubbedConfigFiles, regex includes), `ArchiveChooser.cs`, `PatchCache/`, `CompilerSettings.cs`, allow-list enforcement (L229–247) |
| — *(missing)* **Nexus OAuth + premium download** | **NEW** → `majula-sources::nexus` | Ref: `Wabbajack.Networking.NexusApi/` (`NexusApi.cs`, `OAuth/`, `NexusCredentialSource.cs`, `Endpoints.cs`) |
| — *(missing)* **list repository/gallery client** | **NEW** (M7) | Ref: `Wabbajack.Networking.WabbajackClientApi/Client.cs` — a JSON index of lists; ours is a static-JSON + GitHub-releases model, no server required |

### 5.4 Zed patterns worth borrowing (from the GPUI/Zed codebase)

- **Workspace + panels** — one `Workspace` entity owning panel entities (Zed's `workspace` crate pattern); our panels: Library, Downloads, Install wizard, Games, Settings, Log.
- **Actions & keymap layer** — every user-visible operation is an `Action` struct; keyboard shortcuts and the command palette bind to actions, not to widgets (GPUI `action` module).
- **`ui` crate pattern** — a design-system module of reusable components (Button, Card, Badge, ProgressBar, Modal) styled from one token sheet (§6.1), mirroring `zed/crates/ui`.
- **Background executor for I/O** — all engine calls off the UI thread; UI state updates via entity `update()` from spawned tasks.
- **Theme as data** — colors/spacing as a typed struct swappable at runtime (dark-first; a light theme is a stretch goal, not v1).

---

## 6. UI/UX design

### 6.1 Design system — tokens (from the Modrinth study)

Derived from the owner's screenshot analysis (2026-11-13) + live verification of `#1bd96a` on modrinth.com. Aesthetic inspiration only — not their layout, not their assets.

**Color (dark-first; flat elevation via lightness steps, no drop shadows):**

| Token | Value | Use |
|---|---|---|
| `bg.app` | `#16181d` | app canvas / nav rail |
| `bg.surface` | `#1a1d22` | content regions |
| `bg.card` | `#212429` (hover `#292c32`) | cards, list rows, secondary buttons |
| `bg.inset` | `#0f1115` | inputs, search, "pressed-in" wells |
| `border.hairline` | `rgba(255,255,255,0.06)` | 1px separators |
| `text.primary` | `#e8ebee` | titles, body |
| `text.secondary` | `#9aa1a9` | meta lines, captions |
| `accent` | `#1bd96a` | primary actions, active states, progress, links |
| `accent.fg` | `#0d1117` | text on accent |
| `accent.chip` | `rgba(27,217,106,0.14)` | status chips ("Manual download ready") |
| `danger` | `#f0564f` | hash mismatches, failed items (value ours; Modrinth's red differs slightly) |
| `warning` | `#ffb340` | preflight warnings (ours) |

**Radius:** `sm 8` · `md 12` (cards, buttons) · `lg 16` (window, mod cards) · `pill 999` (chips, search).
**Spacing:** 8px grid — `4 / 8 / 12 / 16 / 24 / 32 / 40`.
**Type scale** (single grotesque family — Inter or a metric-compatible open font bundled for offline use):

| Style | Size/weight | Use |
|---|---|---|
| display | 24 / 700 | screen titles ("Library") |
| h1 | 20 / 700 | card headers, wizard steps |
| h2 | 16 / 600 | section headings, card titles |
| body | 14 / 400–500 | descriptions, meta |
| label | 14 / 600 | buttons, table headers |
| caption | 12 / 400 | timestamps, hashes, sizes |
| mono | 13 / 400 | paths, hashes, log lines |

**Icons:** Lucide-style outline set (~2px stroke, rounded caps), 20–24px nav / 16px inline; solid fills reserved for action/status (play, status dots) — per the Modrinth study.
**Components:** Card (rounded `md`, `bg.card`, hairline border on hover), List row (icon + title/badge + meta + right-aligned action column), Pill chip, Primary button (solid accent, dark text), Secondary button (`bg.card`), Modal sheet, ProgressBar (accent on `bg.inset`), empty states with an illustration + one primary action.

### 6.2 Screen-by-screen spec

1. **Discover / Home** — landing screen. Hero: "Install a modlist" drop zone (drag a `.modfile`/`.wabbajack`, or browse) + "Add a list repository" (M7: a URL that serves a list index). Below: recently opened lists as Modrinth-style cards (icon, title, author, game, version, size of downloads required, last installed). No web dependency — everything local-first.
2. **Library** — grid of known lists (opened/installed): card = game icon block (top ~70%), title, `game • version • #archives` meta line, status chip (Installed v1.0.1 / Never installed / Update available). Click → list detail (see Install wizard entry), kebab `⋮` → open folder, show manifest, remove.
3. **List detail / Install wizard** — the money screen. Left column: description, author, license chip (§3.3 — displayed, never enforced), requirements checklist (game located via registry — DS2 Steam 335300 path resolved; disk space for ~8 GB downloads + ~13 GB install; Nexus account state). Right column: **archive manifest table** — every archive with source chip (Nexus / GitHub / Manual), name, size, and per-item state (✔ hash-verified in cache · ↓ downloading · 🖥 awaiting your click · ✖ failed). Footer: path pickers (downloads dir, install dir — engine enforces WJ-style install-path rules), disk-space meter, **Install** button. During install this screen becomes the live progress view (§7.3).
4. **Downloads queue** — the manual-click UX, full section §7. Shows both lanes (automatic + manual) as one queue with per-item state; the "next manual download" item is pinned with a big **Open download page** button and a "watching your Downloads folder" status line; drag-and-drop zone to adopt a file manually.
5. **Games manager** — table of registry games: detected path, source of detection (Steam/registry/manual), "supported" badge, last used. Add/edit game entries (edits land in the user overlay JSON — §5.3). The shipped registry shows DS2 SOTFS as the flagship entry with its Steam app id 335300, `NBGI\DarkSoulsII` Documents folder, and registry hints, exactly the fields already proven in `known_games.rs`.
6. **Settings** — Nexus account (free/premium state, login via API key or OAuth at M1/M3), downloads folder, concurrency, theme, cache management (hash-cache size, archive cache), data folder location, "open logs".
7. **Log viewer** — append-only, filterable by level; per-install log sessions (the engine's `tracing` setup already writes rotating files via `tracing-appender`); "attach to bug report" button that zips the session log + manifest summary (no paths containing usernames unless the user confirms).

### 6.3 GPUI state-management approach

- **Entities for state:** `Workspace` (root), `LibraryStore`, `DownloadQueue`, `InstallSession`, `GamesStore`, `SettingsStore`, `LogStore` — each a GPUI entity (`Entity<T>`), the Zed pattern (GPUI README: "state management and communication with Entities").
- **Views render from entities:** every screen is a `Render` impl over its store entity or a view entity subscribed to store changes; no unstructured UI-local state for anything the engine owns.
- **Engine bridge:** `majula-core` exposes `InstallSession::spawn(...) -> (JoinHandle, EventReceiver)`; the app's `InstallSession` entity runs a task on GPUI's background executor draining `EventReceiver` and calling `entity.update(...)` per event — coarse-grained updates (≤10 Hz) to avoid re-render storms on 10k-directive progress.
- **Actions:** `install_start`, `download_open_page`, `download_retry`, `game_add`, `settings_save`, … — bound in the keymap; command palette (Zed pattern) surfaces them all with descriptions.
- **Startup:** engine (tokio runtime) starts once in `Application::run`; `Workspace` opens; first-run flow detects games and asks for the Nexus account.

---

## 7. Free-Nexus-user flow (first-class spec)

**Owner's requirement (verbatim intent):** "Without a Nexus Premium account, [the tool] cannot fire off automatic background downloads via the API at high speed. Instead, as it works through the modlist queue, it will trigger your web browser to open each individual mod's download page on Nexus Mods." Per-archive browser-open + local Downloads-folder pickup + hash verification. Premium path as optional accelerator.

**Ground truth on the constraint:** the Nexus API's download-link generation is Premium-gated; Wabbajack's own downloader throws `ManualDownloadRequiredException("Nexus Premium is required for automatic downloads")` for free accounts (`Wabbajack.Downloaders.Nexus/NexusDownloader.cs` L122–124, verified in our fork) and falls back to opening the mod page for a human click. The compliant default is therefore: **the user's browser, the user's click, Nexus's own bot protection** — we never automate, script, or scrape the free-tier download flow (hard rule; §11 Nexus ToS risk).

### 7.1 Queue model

Every archive in the manifest lands in one `DownloadQueue` with a lane:

- **Automatic lane** — Premium API (opt-in, after login), HTTP/GitHub sources, Mega/Drive transfers: normal parallel downloads with progress, resumable (`ResumableDownloader` concept from `Wabbajack.Networking.Http`).
- **Manual lane** — Nexus archives on a free account (and any `ManualDownloader` state, e.g. our two FROST Patreon manuals): state machine below.

### 7.2 Manual-lane state machine (per archive)

```
PENDING → PAGE_OPENED → WATCHING → FOUND → HASH_OK (→ moved into cache/verify pool)
                ↑            │          └→ HASH_MISMATCH → RETRY (offer: re-open page / pick file manually)
                └── user clicked "re-open" 
```

1. **PAGE_OPENED:** the queue opens the mod's Nexus page in the **user's default browser** at the exact file: `https://www.nexusmods.com/{game}/mods/{ModID}?tab=files&file_id={FileID}` (all three values are in the manifest's Nexus state; for our DS2 list: 11 such archives, from `Unpacked Param Files` (1092/3334, 868 KB) to `Comprehensive Texturepack` (1615/6260, 5.34 GB)). The UI card shows: file name, expected size, expected xxHash64 (caption, mono), and a "Open page" button (auto-triggered when the item reaches the head of the queue; the auto-open is a setting — default on, since the owner wants it to "work through the queue").
2. **WATCHING:** a **Downloads-folder watcher** (Windows `ReadDirectoryChangesW` via the `notify` crate; folder = user's configured Downloads dir, default `~/Downloads`) hashes (xxHash64) every new or changed file **as it stabilizes** (size quiescence + rename-out of `.crdownload`/`.part` temp names). The watcher matches hashes against **all pending archives in the queue**, not just the head — a user who batch-downloads five mods in a burst gets credit for all five, in any order (this is the same hash-credit behavior Wabbajack's preflight uses; it removes all ordering pain).
3. **FOUND → verify:** expected hash matches → item flips to ✔, the file is moved (or hardlinked) into the list's downloads folder and enters the standard install pipeline. **HASH_MISMATCH** (wrong file/version downloaded) → card turns `danger` with the expected-vs-got size/hash and buttons: *Open page again* · *Choose file manually* (file picker, hash-checked) · *Skip and continue*.
4. **Aggregate UX:** the Downloads screen shows "Manual downloads 7 / 11" with the next pinned item enlarged; when the queue head is manual and idle > 60 s, a toast offers to re-open the page. Users can also just drop files into the downloads folder or drag them onto the app — the watcher credits them the same way.
5. **Prompted manuals** (`ManualDownloader` state with `Prompt` text — our list's two FROST Patreon items): same state machine, but the card renders the list author's prompt verbatim (e.g. "Download from the FROST Patreon post, then place the file here") with the `Url` opened on click.

### 7.3 Install phase & verification

Once all archives are hash-verified, the installer runs the standard WJ-equivalent pipeline: preflight (game path, disk space, install-path rules) → directives applied in order (streaming extraction with the prototype's fused pipeline, octodiff patches with the patch-result cache, remapped configs rewritten per-user) → **every output file hash-checked against the manifest** (the engine already hard-fails on mismatch — `ThrowOnNonMatchingHash` semantics) → MO2 instance launched/opened at the end with a "What's next" card (README link, antivirus-exclusion hint per the Nordic Souls playbook, launch instructions).

### 7.4 Premium accelerator (optional, not required)

Settings → Nexus account: sign in with a personal API key or OAuth (device flow; reference `Wabbajack.Networking.NexusApi/OAuth`). If the account is Premium and "Use Premium API downloads" is on, Nexus archives move to the automatic lane (full-speed, parallel, resumable). Free account or toggle off → manual lane. **The free flow is the default experience and the marketing headline** — parity with what Wabbajack makes users suffer through, but designed rather than accidental.

---

## 8. Milestones M0–M7

Each milestone: goal, acceptance criteria (observable), rough effort (focused days of one maintainer; calendar ≈ 2–3× that part-time).

**M0 — Foundations & relaunch** *(3–5 d)*
Commit the prototype's ~70 uncommitted files (rebrand + Windows port + DS2 entry) as a clean graft on upstream CLF3 history. Relicense: GPL-3.0 LICENSE + `LICENSE-MIT-CLF3` + rewritten ATTRIBUTION/CONTRIBUTING (new WJ-reference policy) + ACKNOWLEDGMENTS + MODLIST-POLICY template. Create `himanshudbg/majula` public repo, push full history (132 CLF3 commits preserved), fix `Cargo.toml` repository URL. Split into the workspace crate skeleton (§5.1). CI: `windows-latest` + `ubuntu-latest` build/test/clippy/fmt + cargo-audit. Fix the 9 Windows test failures (7z PATH ×2 — locate `C:/Program Files/7-Zip/7z.exe`; MOMW unix-path ×6 — cfg-gate or relocate to a `momw` feature; DirectXTex COM ×1 — mark `#[ignore]` on machines without the GPU interface). GPUI spike: hello-world window builds & runs on Windows with `gpui = "0.2.2"`.
**Accept:** green CI both OSes; `cargo deny` clean; repo public with all license/attribution docs; GPUI spike screenshot in the PR.

**M1 — Engine crates ported** *(5–10 d)*
Move prototype modules into `majula-core`/`majula-sources`/`majula-games` per the migration table (§5.3); `majula-cli` regains `install/info/games/hash-file` verbs. Data-driven `games.json` registry live (DS2 entry migrated from `known_games.rs`; the 13 existing entries carried over). Windows Credential Manager keyring wired.
**Accept:** `majula install "DS2-Modpack-by-Bonkfyre-1.0.1.wabbajack" --downloads … --output …` completes on a clean Windows sandbox with the manual lane (engine-level, CLI prompts) and all 15 archives hash-verified; `majula games list` shows DS2 detected from Steam.

**M2 — `.modfile` v1 + `.wabbajack` interop** *(4–7 d)*
Implement §4: marker, manifest v1 (FormatVersion/License/BlobIndex), zip container writer, `convert` verb, blob-index verification on load.
**Accept:** `majula convert` on the showcase list → `.modfile`; installing the `.modfile` yields an output tree whose complete xxHash64 set is **identical** to the `.wabbajack` install (automated comparison script in `tests/`); unknown-directive and wrong-FormatVersion files produce clear errors before any download.

**M3 — GPUI shell v1** *(10–20 d — the learning-curve milestone)*
`majula-app` with the §6.1 design system (token sheet + component kit), the §6.2 screens (Discover, Library, List detail/Install wizard, Downloads queue with the full §7 manual-lane UX, Games, Settings, Log), §6.3 state architecture. Engine driven entirely from the GUI.
**Accept:** a non-technical user can install the showcase `.wabbajack` end-to-end from the GUI alone — including the free-Nexus manual lane (browser opens, Downloads folder watched, hash mismatch handled) — with zero terminal usage; design review against the Modrinth reference screenshot passes the owner's eye.

**M4 — Parity proof & showcase list** *(3–6 d)*
This is the credibility milestone. Install the showcase list into a sandbox via the GUI; hash-compare **every output file** against the known-good instance (`H:/Games/DS2 Modlist` — the source MO2 instance the list was compiled from; equivalently against a WJ-fork-installed reference).
**Accept:** all 10,616 output files xxHash64-match the known-good instance (incl. the 79 patched files — the 74 recolored HUD DDS + the 158 MB `nvngx_dlssnr.dll` reconstructed from the 16.3 MB delta); install time and peak RAM recorded as the baseline numbers for marketing; any deviation documented root-caused.

**M5 — The compiler** *(15–30 d — the biggest single milestone)*
`majula-compiler`: MO2 instance → `.modfile`, porting the WJ compile cascade (reference `Wabbajack.Compiler`: VFS-index the MO2/downloads/game roots; meta-file cascade `directURL`/`manualURL`+`prompt`/Nexus mod-file state; MakeStack matching — ignore rules → DirectMatch → tag includes → binary-patch-vs-closest-archive → regex includes → NoMatch abort; stubbed-config path remapping; patch economy; download allow-list enforcement with our own `allowlist.yml` seeded from WJ's `ServerWhitelist.yml` conventions). Compile wizard in the GUI (settings, ignore lists, metadata, progress, NoMatch report screen).
**Accept:** compiling the DS2 source instance produces a `.modfile` that (a) installs back to a tree hash-identical to the source instance, (b) contains **zero mod bytes** from third-party archives (only hashes, patches ≤ size of modified files, inline configs/author-owned files), (c) is comparable in size to the WJ-produced 42.4 MB artifact. NoMatch flow demonstrated on a deliberately dirty instance.

**M6 — Distribution & trust kit** *(3–5 d)*
Per §9: release pipeline (tag → CI builds → SHA-256 manifest → GitHub release), VirusTotal submission workflow, SECURITY.md + private vulnerability reporting, signing step (even if initially self-signed/deferred — the pipeline must have the hook), README marketing pass (§10), user docs (install guide, list-author guide for `.modfile`).
**Accept:** a tagged release exists with hashes, a VT report link, and docs a stranger can follow cold; `winget` package submitted (stretch).

**M7 — Community launch & repositories** *(5–8 d + ongoing)*
List-repository support (M7 scope: read a static JSON index + download `.modfile`s from any HTTPS host / GitHub releases — the "custom repository" model from the WJ wiki, minus their server); the Discover screen gains a repo browser. Launch per §10: Reddit posts, Nexus forums thread, Discords, Show HN.
**Accept:** v1.0.0 announcement post is live with an honest comparison table; at least one non-owner user installs the DS2 list from a repository and reports success; issue triage cadence established.

**Total rough effort:** ~48–91 focused days → **3–6 months part-time** to M7; M0–M4 (the "installs our list beautifully" product) is ~25–48 focused days, reachable in weeks if full-time.

---

## 9. Security & trust kit

A mod-list installer asks users to run an unsigned binary that downloads gigabytes and writes into their game folders — trust is the product. Day-1 posture:

- **Public repo from the first commit** (M0). No private phase. GPL makes this the natural state; see §11 for the discipline note.
- **Reproducible-ish builds:** `rust-toolchain.toml` pinned; `Cargo.lock` committed; `cargo build --locked --release` from a clean checkout is the documented build; CI builds every tag twice (two runners) and records both SHA-256s — matching hashes at the same commit+toolchain is our reproducibility signal (full bit-reproducibility with GPUI/embedded resources is a stretch goal, not a claim we make).
- **Per-release artifact manifest:** every release asset gets a `SHA256SUMS.txt` (and a `SHA256SUMS.sig` once signed) — mirroring how we verified our own list artifact (`65E0F05A…`) — plus the exact commit hash and `Cargo.lock` hash it was built from.
- **VirusTotal workflow:** on every release, submit the binary via the VT API (free tier, 4 req/min is enough for one artifact); link the analysis URL in the release notes. This directly addresses the modding-community's "new installer = malware?" reflex, and gives us early false-positive detection.
- **Code-signing roadmap:** unsigned first (SmartScreen "More info → Run anyway" + documented hash verification), then **Azure Trusted Signing** (~$9.99/month, individual-friendly, the current standard for OSS Windows apps) or a Certum open-source cert. Signing enters the release pipeline in M6 (even if initially producing only a self-signed marker for upgrade-path testing). Never let unsigned → signed transitions break auto-update paths (updater verifies hashes regardless).
- **SECURITY.md:** coordinated disclosure policy + GitHub private vulnerability reporting enabled; supported versions table; `cargo audit` in CI (already planned M0) for dependency CVEs.
- **App-level hardening:** downloads verified by xxHash64 before use (inherent to the format); no script/executable execution during install (directives only write files — same as WJ; this invariant is documented as a format guarantee); secrets in Windows Credential Manager (§5.2); no telemetry in v1 (if ever added: opt-in, documented).
- **Docs:** a "Why you can trust this" page: who we are, what the binary does (network calls enumerated: Nexus API + the mod sources' hosts only), how to verify hashes, how to build it yourself.

---

## 10. Marketing & positioning

**README pitch (draft headline):** "Majula is a free, open-source modlist installer — a modern alternative to Wabbajack. It installs your existing `.wabbajack` modlists today, and its own `.modfile` lists tomorrow. Rust-fast, GPU-rendered UI, and a first-class experience for free Nexus accounts: no Premium required, just your browser and a few clicks."

**Honest comparison table (README centerpiece — honesty is the differentiator):**

| | Wabbajack | Majula |
|---|---|---|
| License | GPLv3 | GPLv3 |
| Language / runtime | C# / .NET 10 | Rust / none (single binary) |
| UI | Avalonia | GPUI (Zed's GPU-accelerated framework) |
| Installs `.wabbajack` | ✔ | ✔ (day one) |
| Own format | `.wabbajack` (7z, no version field) | `.modfile` (zip, explicit `FormatVersion`, list-license field) |
| Free Nexus account | Manual click-through (works; exception-driven UX) | Manual lane designed as the primary flow (browser-open, watched Downloads folder, hash-verified pickup) |
| Premium Nexus | Full-speed API | Same (optional accelerator) |
| Games | ~50, compile-time registry, Bethesda-centric | Data-driven `games.json`; **DS2 SOTFS first-class**; community-extensible without rebuilds |
| Compiler | Mature, battle-tested | v1.0 ships at M5 (ported from WJ's design) |
| Gallery / CDN / modlist hosting | ✔ (mature, curated) | Local-first; static-JSON repositories (M7); no hosting of mod bytes ever |
| Platforms | Windows, Linux | Windows first (Linux engine-ready) |

**Why (the launch-post skeleton):** ① *Free-user-first* — the headline feature nobody else designs for; ② *Rust speed & safety* — streaming installs with bounded memory (the prototype's mimalloc-tuned extractor exists because of real RSS problems WJ-style tools hit); ③ *a 2026 UI* — GPU-accelerated, dark, rounded, fast; ④ *multi-game by data* — DS2 SOTFS is a first-class citizen (a game stock Wabbajack cannot install at all — the entire reason this project exists); ⑤ *open, verifiable builds*.

**Launch channels (in order):**
1. r/DarkSouls2 + DS2 modding Discords (FROST community adjacency, respectfully) — the flagship-list audience.
2. r/skyrimmods, r/skyrimmodsX, r/Morrowind (OpenMW lists via CLF3 lineage), r/modding — the Wabbajack-literate audience; frame as "alternative", never "replacement", credit WJ generously.
3. Nexus Mods forums (the Wabbajack subforum is the natural place for a "here's a new installer" thread; their ToS doesn't forbid discussing alternatives).
4. r/rust + Hacker News ("Show HN: I'm building a Wabbajack alternative in Rust with Zed's GPUI") — the tech story recruits contributors.
5. The Wabbajack Discord itself, as a courtesy heads-up (they're friendly OSS folks; halgari's own attitude to game-addition PRs is "feel free to open a PR" — good-faith neighbors make better ecosystems).

**Assets to prepare (M6):** logo + brand mark, 60-second install GIF (the manual lane *is* the demo), screenshots of the DS2 list installing, the comparison table, a "verify our hashes" how-to.

---

## 11. Risk register

| # | Risk | Sev. | Mitigation |
|---|---|---|---|
| 1 | **GPUI-on-Windows maturity / pre-1.0 churn.** GPUI is pre-1.0 with breaking changes between versions (stated in its README); it is tied to Zed's development. | High | Strong counter-evidence: Zed ships stable Windows builds (1.28.1, 2026-11-11; Win32 + DirectWrite backend, DX11 requirement — documented). Mitigations: pin `gpui = "=0.2.2"` (crates.io, 2025-10-22) and only bump deliberately; evaluate a Zed-tag git pin if a Windows fix lands upstream first; keep the engine 100% GPUI-free (§5.2) so worst-case we re-shell the UI; budget time to contribute Windows fixes upstream (owner mandates GPUI-only — this is an accepted, watched risk, not a surprise) |
| 2 | **Scope: compiler + new UI + new format simultaneously.** The prototype lacks a compiler entirely; M5 is a big port of a complex C# subsystem. | High | Milestone gates keep each slice shippable; the **WJ fork remains the list compiler until M5 lands** (§12) so the modlist pipeline never blocks on our app; M0–M4 delivers a useful installer with zero compiler work; `x-` directive extension point avoids format churn |
| 3 | **Single-maintainer bandwidth.** ~3–6 months part-time to v1.0; bus factor 1. | High | Public repo + clean docs from M0 to recruit contributors; the r/rust launch targets exactly the people who'd contribute a GPUI app; milestone acceptance criteria make progress legible; deferral list already drawn (MOMW/TTW/collections stay out of v1 UI) |
| 4 | **GPL source-publication discipline.** Every distributed binary (including Patreon-attached interim builds) must come with corresponding source. | Med | Public-from-day-one repo makes this automatic; release process tags the exact commit; CONTRIBUTING states "no private builds of this project are distributed"; the *Wabbajack fork* used for compiling lists must also keep its source public (it does — branch `dark-souls-2-support`, and our fork's repo is public) |
| 5 | **Nexus Mods ToS.** Automating free-tier downloads (scraping, bot-driving the site) would violate ToS and get keys blocked. | Med | Hard rule in CONTRIBUTING + engine architecture: the manual lane **only** opens the user's browser and watches a local folder — never drives, scrapes, or retries against nexusmods.com; API usage (metadata, premium links) goes through the official documented API exactly as MO2/Vortex/WJ do (WJ wiki FAQ documents this as the sanctioned channel); polite rate limits (WJ's `RateLimiter` semantics) |
| 6 | **Trademark friction ("Wabbajack alternative").** | Low | Nominative fair use only (§3.4): comparison language, their name in text, never logos/endorsement; generous GPL-credit; courteous heads-up post in their Discord before launch |
| 7 | **Modlist-data licensing ambiguity** (FROST-owned components in the showcase list; "no selling modlists" unenforceable by code). | Med | §3.3 policy model (CC BY-NC-SA default notice, author-overridable, displayed not enforced); the showcase list's own FROST-attribution and permission notes travel with it (phase-3b work); app takes no legal position — documents norms |
| 8 | **AV false positives on unsigned Rust binaries** (the classic mod-tool problem — Nordic Souls literally tells users to uninstall Norton). | Med | §9 trust kit: VT workflow, hashes, docs; signing via Azure Trusted Signing when budget allows |
| 9 | **Windows test/env debt from the prototype** (9 failing tests, Linux-only CI, placeholder repo URL). | Low | All scheduled in M0 acceptance; none touch the installer core (VERDICT) |
| 10 | **Upstream CLF3 divergence** (SulfurNitrice keeps moving; our fork is already 70 files ahead). | Low | We are a hard fork with a new identity; document the pin commit; cherry-pick upstream fixes opportunistically |

---

## 12. Migration & repo plan

**Recommendation: new public repo, seeded with the prototype's full history — do not "evolve" `ds2-modpack-app` in place.**

1. **Create `github.com/himanshudbg/majula`** (public, GPL-3.0, per §2/§3).
2. **Push the existing prototype repo's history unchanged** (the 132-commit CLF3 lineage — MIT compliance *wants* the history; GPL wants the future public). Then commit the currently-uncommitted ~70-file rebrand/port work as one clean graft commit ("Windows port + DS2 game entry + rebrand to Majula"), followed by the relicense commit (LICENSE GPL-3.0, LICENSE-MIT-CLF3, docs) and the workspace-split commit. Nothing is squashed; `ATTRIBUTION.md`'s grafted-history claim becomes true.
3. **Why not evolve in place:** `ds2-modpack-app` carries dev logs, `tmp-verify/`, `clf3-upstream/` clone, and a placeholder identity (its `Cargo.toml` repository URL already points at `himanshudbg/ds2-modpack-app`); a fresh repo with the same history gives a clean face for marketing while `ds2-modpack-app` remains the private working directory until the public flip, after which it is archived (a pointer README, no deletions).
4. **CLF3 lineage handling:** preserved forever via history + `LICENSE-MIT-CLF3` + `ATTRIBUTION.md`; the old "never reference WJ C#" rule is replaced by the GPL acknowledgment model (§3.1); an issue/PR is opened upstream to CLF3 as a courtesy note that we forked (MIT etiquette).
5. **The modlist pipeline meanwhile:** the **Wabbajack fork stays the compiler** for `DS2 Modpack by Bonkfyre` until M5 — it is proven (v1.0.1 compiled clean in 11 s, zero NoMatch) and its branch (`dark-souls-2-support`, +18-line `DarkSoulsII` registry patch) is exactly the reference `majula-compiler` will port. The private `himanshudbg/ds2-modpack-bonkfyre` repo remains the list's distribution stash (patched WJ build zips + `.wabbajack`), with the standing note that the *Majula app* replaces the Wabbajack-named zips as the subscriber download once M4 proves parity. `wabbajack-fork/` on disk is reference + compiler; it is never modified by Majula work (its GPLv3 source must stay buildable as the fallback).
6. **What happens to each on-disk asset:** `ds2-modpack-app/` → migrates into the new repo (then archived locally); `wabbajack-fork/` → untouched compiler/reference until M5; `DS2 Modlist/` (the instance) → the M4 known-good hash oracle and M5 compile source; `.frostfx-research/` → stays private research (never committed — it contains community/workflow notes, not app material).

---

## 13. Immediate next actions (post-approval checklist)

Ordered; each is small and independently verifiable.

1. **Approve the name** (recommendation: Majula; §2) and the GPL-3.0 + modlist-policy resolution (§3). If Bonfire preferred instead: proceed identically, adjust strings only.
2. **Commit the prototype working tree** (70 files) on top of CLF3 HEAD `a6f332a` — nothing else before this; the work is currently uncommitted and at risk.
3. **Create the public repo + push history** (§12 steps 1–2); set repo topics (`rust`, `gpui`, `modding`, `mod-organizer-2`, `wabbajack-alternative`, `dark-souls-2`), branch protection on `main`.
4. **Relicense commits:** GPL-3.0 LICENSE, LICENSE-MIT-CLF3, rewritten ATTRIBUTION/CONTRIBUTING, ACKNOWLEDGMENTS, MODLIST-POLICY.md, SECURITY.md stub.
5. **Workspace split** into the §5.1 crate skeleton (moves only, no behavior changes; `cargo test` still 290+ passing).
6. **CI:** GitHub Actions `windows-latest` + `ubuntu-latest` (build/test/clippy/fmt + cargo-audit); fix or gate the 9 Windows test failures; correct the placeholder `repository` URL.
7. **GPUI spike on Windows:** `gpui = "=0.2.2"` hello-world window + one Modrinth-token card; record build time, binary size, and any Windows blockers as the risk-register baseline (this is the first real data point for Risk #1).
8. **Windows Credential Manager keyring** port (small, high-trust-value).
9. **`games.json` data-driven registry** migration (DS2 entry first; carry the 13 existing entries).
10. **Update the private list-repo release notes** (phase-3b standing note) to point future subscribers at the Majula app once it exists — no distribution change yet.

Then: M1 begins (engine crate moves + CLI parity on the showcase list).

---

## Appendix A — Research citations (fetched/verified 2026-11-13)

**GPUI / Zed**
- GPUI site & intro: https://gpui.rs/
- GPUI crate (v0.2.2, 2025-10-22, Apache-2.0, Windows in default features via `windows-manifest`; edition 2024): https://crates.io/api/v1/crates/gpui · https://crates.io/crates/gpui
- GPUI README (pre-1.0/breaking-changes warning; Windows = Win32 + DirectWrite, no features required; entities/views/elements model; actions; async executor; `gpui_platform` for standalone apps): https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md
- Zed stable ships Windows builds (1.28.1, 2026-11-11; DX11 requirement; winget `ZedIndustries.Zed`): https://zed.dev/download · https://zed.dev/docs/windows.md
- Building Zed/GPUI apps on Windows: https://zed.dev/docs/development/windows.md
- Zed docs index (patterns: actions, command palette, panels, themes): https://zed.dev/docs/llms.txt
- Zed UI crate (design-system-as-crate pattern): https://github.com/zed-industries/zed/tree/main/crates/ui

**Modrinth (design reference)**
- Discover page (brand accent `#1bd96a` verified via `meta-theme-color`): https://modrinth.com/discover/mods
- Owner's screenshot reference: `local://pasted-image-bdb262d96eda0982.png` (analyzed 2026-11-13: dark `#1a1d22`-family surfaces, cards `#292c32`, radius 12–16 px, pill chips, Inter-like grotesque, 8px grid, flat lightness-step elevation, Lucide-style outline icons — aesthetic inspiration only)

**Wabbajack (parity target & policies)**
- Wiki root / FAQ / license (GPLv3 code; modlists CC BY-NC-SA; paywalling prohibited, donations fine; Nexus API = official channel): https://wiki.wabbajack.org/
- Compiler mechanics, meta files, allow-lists, manualURL/prompt, Nexus free-vs-premium: verified against our fork + documented with wiki/code URLs in `H:/Games/DS2 Modlist/.frostfx-research/wabbajack-plan.md` (key wiki pages: Pre-Compilation, Compilation Settings, Meta Files, Compilation Issues FAQ, Keeping the Game Folder Clean; allow-list: https://raw.githubusercontent.com/wabbajack-tools/opt-out-lists/master/ServerWhitelist.yml)
- GameRegistry (no FromSoftware games; game-addition pattern = 1+17 lines per PR #3032): https://github.com/wabbajack-tools/wabbajack/blob/main/Wabbajack.DTOs/Game/GameRegistry.cs
- Reference implementation on disk: `H:/Games/wabbajack-fork/` (v4.3.2.2 + `DarkSoulsII` patch, branch `dark-souls-2-support)`

**Nexus Mods**
- Free-tier constraint (Premium required for API download links; fallback = open mod page for a human click): verified in fork source `Wabbajack.Downloaders.Nexus/NexusDownloader.cs` L122–124, L157–159; official API docs (linked from WJ wiki FAQ): https://app.swaggerhub.com/apis-docs/NexusMods/nexus-mods_public_api_params_in_form_data/1.0

**Engine seed & showcase**
- CLF3 upstream (MIT): https://github.com/SulfurNitride/CLF3 (pinned commit `a6f332a65d112ee22fb3abb9700e6e4f8512a32d`)
- Prototype assessment: `H:/Games/ds2-modpack-app/VERDICT.md` · `ATTRIBUTION.md` · `Cargo.toml` · `src/` survey (2026-11-13)
- Showcase list anatomy: `H:/Games/DS2 Modlist/.frostfx-research/wj-phase3b-private-recompile.md` (artifact `H:/Games/wabbajack-fork/output/DS2-Modpack-by-Bonkfyre-1.0.1.wabbajack`, SHA-256 `65E0F05A25F19AC4A4291B3FE16B95C3052BF8D677C7113D4499C59554E3BD0B`)

**Name-collision checks (2026-11-13)**
- `majula` free on crates.io: https://crates.io/api/v1/crates/majula (404) · GitHub search: no in-domain collisions (top: hazevauks/MajulaRecomp — DS2 Switch recomp, different product; RustCityRS/rs-majula — RuneScape server)
- Rejected: `modwright` (active npm package, game-modding MCP server: github.com/jtrachtenberg/modwright-community) · `netch` (17.7k-star C# proxy: github.com/netchx/netch) · `estus` (crates.io squatted: "Reserved name for future use") · `bonfire`/`caravan`/`coffer` (crates.io names occupied — usable only with suffixed crate names)

---

## Appendix B — Machine-state inventory (what exists right now)

| Asset | Path | State |
|---|---|---|
| Engine seed (CLF3 fork) | `H:/Games/ds2-modpack-app/` | Builds on Windows (release 8 m 22 s); 318 tests (290 pass / 9 env / 19 ignored); DS2 entry + all 4 core directive handlers present but **uncommitted** (70 files); MIT + attribution in order |
| WJ reference + compiler | `H:/Games/wabbajack-fork/` | v4.3.2.2 + DarkSoulsII patch, builds, **compiled our list**; ~40 C# projects; GPLv3 |
| Showcase list | `H:/Games/wabbajack-fork/output/DS2-Modpack-by-Bonkfyre-1.0.1.wabbajack` | 42.4 MB, SHA-256 `65E0F05A…BD0B`; 15 archives (11 Nexus / 2 HTTP / 2 manual); 10,616 directives incl. 79 patches; GameType DarkSoulsII |
| Known-good instance | `H:/Games/DS2 Modlist/` | The M4 hash oracle + M5 compile source |
| Research corpus | `H:/Games/DS2 Modlist/.frostfx-research/` | wabbajack-plan.md, wj-phase3b-private-recompile.md (+ phases 0–3) |
| List distribution | `himanshudbg/ds2-modpack-bonkfyre` (GitHub, private) | Release v1.0 with patched-WJ zips + list; subscriber-facing notes already anticipate the Majula app |

*End of plan. Nothing has been implemented; this document is the deliverable. Next gate: owner approval of §2 (name), §3 (licensing resolution), and §8 (milestone scope).*

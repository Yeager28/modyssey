# Security Notes

Accepted-risk register for dependency advisories that Modyssey cannot currently
patch, plus the reasoning behind each acceptance. Reviewed at every milestone.

For reporting vulnerabilities in Modyssey itself, see [SECURITY.md](SECURITY.md).

## Dependency advisories — accepted residuals (M1, 2026-11-13)

`cargo audit` baseline at M0 was **41 vulnerabilities** (2024-era CLF3
dependency tree). The M1 dependency pass (lockfile refresh + targeted major
bumps: rusqlite 0.38→0.42/libsqlite3-sys 0.40, zip 8.4→9.5, plus full
`cargo update` of the transitive tree — crossbeam-epoch, h2, openssl 0.10.82,
rustls, rustls-webpki, quinn-proto, zstd, webbrowser, webpki-root-certs, and
others) reduced this to **4 findings (3 distinct advisories)**, all
upstream-pinned and documented below. All 31 baseline yanked/unsound warnings
are also cleared.

### RUSTSEC-2024-0437 — `protobuf` 3.5.1 (crash via uncontrolled recursion)

- **Chain:** `steam-vent 0.5.0` → `steam-vent-proto-common 0.5.1` →
  `protobuf =3.5.1` (exact pin).
- **Why unfixable:** `steam-vent` 0.5.0 (the newest published release)
  hard-pins `protobuf =3.5.1`; the patched version (≥3.7.2) cannot be selected
  without forking steam-vent and rewriting its generated protocol bindings
  (protobuf 3.7 changes the codegen surface).
- **Exposure:** LOW. The advisory is a *crash* (stack overflow DoS), not
  memory corruption or RCE. Modyssey uses steam-vent to log into the user's
  own Steam account and parse protocol data from Steam's own servers — a
  trusted peer, not attacker-controlled input. A malicious Steam server would
  only crash the login session.
- **Revisit when:** steam-vent publishes a release on protobuf ≥3.7.2.

### RUSTSEC-2026-0194 & RUSTSEC-2026-0195 — `quick-xml` 0.30.0 (quadratic runtime / unbounded namespace allocation)

- **Chain:** `eframe 0.31.1` → `egui-winit 0.31.1` → `accesskit_winit 0.23.1`
  → `accesskit_unix 0.13.1` → `atspi-common 0.6.0` → `zbus-lockstep 0.4.4` →
  `zbus_xml 4.0.0` → `quick-xml 0.30.0` (patched: ≥0.41.0).
- **Why unfixable:** the old atspi/accesskit line is pinned by eframe 0.31
  (the crate's last 0.31.x release); the newer accesskit_winit line (which
  moves to atspi-common 0.14 with no zbus_xml runtime dep) requires
  egui-winit ≥0.32. Modyssey's egui GUI is scheduled for full replacement by
  GPUI at milestone M3 — upgrading the egui stack now would be throwaway work
  on code that is being deleted.
- **Exposure:** LOW, and Linux-GUI-only. These crates never compile into
  Windows builds (target-gated `cfg(unix)` accessibility stack). On Linux the
  parsed XML is DBus/AT-SPI introspection data from the local accessibility
  bus — system-provided, not network input. The advisories are DoS-class
  (slow/large allocations), not memory-safety issues.
- **Revisit when:** the egui GUI is removed at M3 (residual disappears with
  it), or if M3 slips and an egui bump becomes necessary for other reasons.

### RUSTSEC-2023-0071 — `rsa` 0.9.10 (Marvin Attack: timing sidechannel in private-key decryption)

- **Chain:** `mega 0.8.0` (`^0.9.6`) and `steam-vent 0.5.0` (`^0.9.10`) →
  `rsa 0.9.10` (patched: ≥0.10.0).
- **Why unfixable:** both parents require the 0.9 line; rsa 0.10 is a
  semver-incompatible major. No newer mega or steam-vent release exists.
- **Exposure:** LOW. The Marvin attack requires an attacker on the *same
  machine* taking high-precision timing measurements across many RSA private
  key operations. Modyssey's RSA use is Mega key unwrapping (the user's own
  Mega account, decrypting Mega's server responses) and Steam authentication
  (steam-vent) — the private keys are the local user's own, and an attacker
  with local code execution sufficient to mount a timing attack already has
  everything worth stealing.
- **Revisit when:** mega or steam-vent publishes a release on rsa ≥0.10.

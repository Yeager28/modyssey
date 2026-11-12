# Modlist data policy

Modyssey installs modlists. A modlist file (`.wabbajack` today, `.modfile`
soon) is **data**, not program code — the GPL license of the Modyssey
application does not and cannot govern it. This document states the policy
we ask list authors to follow. It is modeled on Wabbajack's long-standing
modlist policy.

## The default

Modlists built with Modyssey default to the
**Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International
(CC BY-NC-SA 4.0)** notice for the list's data: its structure, ordering,
configs, patches, and metadata *as a collection*. The `.modfile` manifest
format carries a `License` field; when a list declares none, the app
displays "CC BY-NC-SA 4.0" as the default notice.

## Author override

The list author may declare any license they have the right to declare for
their own list, via the manifest's `License` field or a license document
shipped with the list. The default is a floor of good manners, not a
ceiling.

## Displayed, never enforced

Modyssey **displays** the declared license on the list detail screen and in
the install wizard. It does **not** refuse to install anything. Code-level
enforcement of data licensing is trivially strippable and hostile to the
modding ecosystem; these norms live here, in the community, and in each
author's choices.

## The two hard asks (the Wabbajack precedent)

1. **Modlists are free to share.** Access to a list must never be
   payment-gated or paywalled.
2. **Donations are a "thank you", not a purchase.** A donation link on your
   list's page is fine; requiring payment to receive the list is not.

## What a modlist never contains

A Modyssey/Wabbajack-style modlist contains hashes, source pointers, binary
patches, and small config files — never third-party mod archives
themselves. The mods stay where their authors published them; the list
points, it does not redistribute.

## Trademark note

"Wabbajack" is used nominatively throughout this project's documentation to
identify the incumbent tool for comparison and interop
("installs `.wabbajack` modlists", "a Wabbajack alternative"). No
affiliation or endorsement is implied.

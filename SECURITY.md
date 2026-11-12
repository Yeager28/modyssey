# Security policy

## Reporting a vulnerability

Please use **GitHub's private vulnerability reporting** for this repository
(Security tab → "Report a vulnerability"), which is enabled. If you cannot
use it, contact the maintainer through the GitHub profile
([Yeager28](https://github.com/Yeager28)).

**Please do not open public issues for security problems.**

## Coordinated disclosure

- Reports are acknowledged as quickly as the maintainer can manage.
- We coordinate a fix and a disclosure timeline with you; credit is given
  in the release notes unless you prefer otherwise.
- Interim advisories are published if a fix takes longer than expected.

## Scope notes

Modyssey downloads archives from the sources a modlist points at and
verifies every byte by xxHash64 before use; the install pipeline writes
files only and never executes downloaded content. Reports about a specific
*modlist's* contents belong with that list's author, not this project.
Suspected malware in a mod/archive hosted elsewhere should be reported to
the host (e.g. Nexus Mods).

## Supported versions

The project is pre-1.0: the default branch is the only supported line.

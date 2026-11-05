# Asset Sourcing & Tracking

This file defines the rules for sourcing, downloading, using, and tracking external assets in this repository.

These instructions apply to both human contributors and AI coding agents.

## Core Rule

**Every externally sourced asset MUST be downloaded to the repository's `assets/` directory before it is used anywhere in the project.**

Do not reference, hotlink, or directly use an external asset from its original URL.

The `assets/` directory is the source of truth for externally sourced materials.

If an asset needs to exist elsewhere in the project:

1. Find an appropriate asset that may legally be used.
2. Verify its source and license.
3. Download the original asset into `assets/`.
4. Record it in the **Asset Registry** below.
5. Copy the asset from `assets/` to the required project location.
6. Use the copied version in the project.

Never download an external asset directly into its final project location.

---

## Allowed Assets

Prefer assets that are clearly licensed for reuse.

Suitable sources include:

* CC0 / Public Domain
* MIT
* Apache-2.0
* BSD licenses
* Creative Commons licenses compatible with the project's use
* Open-source projects with explicit licenses
* Official asset libraries that explicitly permit the intended use
* Assets created specifically for this project

Always prefer **CC0, Public Domain, or similarly unrestricted assets** when multiple suitable options exist.

A publicly accessible file is **not automatically public domain or free to use**.

---

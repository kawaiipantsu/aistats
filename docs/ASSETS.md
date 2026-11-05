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

## Asset Registry

| Asset | Source | License | Location |
|---|---|---|---|
| `banner.svg` | Created directly for this project (original SVG markup, no external source) | Project license (MIT) | `assets/banner.svg`, copied to `.github/images/banner.svg` for README use |
| Bricolage Grotesque (variable Latin WOFF2) | Google Fonts; [upstream](https://github.com/google/fonts/tree/main/ofl/bricolagegrotesque) | SIL OFL 1.1; adjacent `bricolage-grotesque-OFL.txt` | `assets/fonts/bricolage-grotesque-latin.woff2` |
| Space Grotesk (variable Latin WOFF2) | Google Fonts; [upstream](https://github.com/google/fonts/tree/main/ofl/spacegrotesk) | SIL OFL 1.1; adjacent `space-grotesk-OFL.txt` | `assets/fonts/space-grotesk-latin.woff2` |
| DM Sans (variable Latin WOFF2) | Google Fonts; [upstream](https://github.com/google/fonts/tree/main/ofl/dmsans) | SIL OFL 1.1; adjacent `dm-sans-OFL.txt` | `assets/fonts/dm-sans-latin.woff2` |
| README screenshot | Screenshot of the project HTML report interface; all displayed figures and pseudonyms replaced with fictional sample data | Project license (MIT); embedded font licenses as below | `assets/screenshot.png` |
| Fun stats robot and social card | Original SVG artwork and card composition created for this project | Project license (MIT); font licenses as above | `assets/fun-mascot.svg`, `assets/ai-stats-social-v1.png`, `contrib/render-fun-social.mjs` |

Fonts were downloaded on 2026-10-31. Exact CSS/font/license source URLs and
SHA-256 hashes are in `assets/fonts/sources.json`. The original font bytes are
embedded in exports, alongside the full licenses. No font CDN is contacted
when a report is generated or viewed. The bundled PNG is rendered from the
original vector and fonts with `node contrib/render-fun-social.mjs` (Node 22+
and Chromium); these tools are not runtime dependencies of the CLI.

---

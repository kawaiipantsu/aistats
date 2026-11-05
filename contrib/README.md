## `contrib/`

```text
contrib/
```

Contains supporting material that contributes to the project but does not belong directly in the primary application source.
Depending on the project, this may include:

* Integration helpers
* Example configurations
* Packaging helpers
* Service definitions
* Third-party integration files
* Deployment examples
* Community-contributed utilities
* Supporting scripts

Files placed here should have a clear purpose related to supporting the project.

---

## ai-stats contrib contents

- `config.example.yaml` — an annotated example of every `~/.config/ai-stats/config.yaml` key.
- `cron/ai-stats-sync` — an example `/etc/cron.d` entry for unattended periodic `ai-stats sync`.
- `cron/ai-stats-fun-stats` — an example daily root crontab line for refreshing
  a hosted fun-stats page, with a lock and log file.
- `cron/ai-stats-refresh-html` — the publishing wrapper used by that entry.
  It builds in a private staging directory beside the destination and renames
  complete files into place, keeping the previous page intact if export fails.
  Optional SSH flags follow the output path and public URL and are passed to
  ai-stats unchanged; keep actual hosts and key paths in the deployed crontab.
- `render-fun-social.mjs` — regenerates the bundled 1200 × 630 social PNG using
  the vendored fonts and original mascot SVG. Run with Node 22+ and Chromium;
  optionally set `CHROMIUM` to the browser executable. No npm packages or
  network access are needed. Rebuild the Rust binary after changing the image.

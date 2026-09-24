# Proposal

## Why

Screenshots make many bug reports understandable, and the owner wants them per project,
re-encoded to lossy WebP. Images are the most dangerous input Kohaku takes (decoder bugs,
decompression bombs, metadata leaks, content sniffing), so they get their own change
(design §6 Screenshots, §15 item 9).

## What Changes

- Multipart upload on the HTML form of projects with `screenshots_enabled`: text first, at
  most 3 files of 8 MiB, 25 MiB per request, 4 concurrent submissions.
- Header checks, memory-safe decoding with limits, downscaling past 2560 px, lossy WebP q80
  via the libwebp encoder only; 2 concurrent image jobs.
- Storage as BLOBs; per-project and per-instance quotas (optional settings, defaults 256 and
  1024 MiB).
- Public serving through the views with a sandbox CSP; admin viewing and deletion; spam
  rejection deletes screenshots.

## Capabilities

### New Capabilities

- `screenshots`: uploads, checks and re-encoding, quotas, serving.

### Modified Capabilities

- `configuration`: the two quota settings.
- `audit-log`: `screenshot.delete`.
- `moderation`: screenshot URLs in public detail.

## Non-goals

- Screenshots through the JSON API, other formats (GIF, AVIF, HEIC), video, image editing,
  thumbnails beyond the one downscale.

## Security considerations

- **Attacker: decoder exploits.** Only memory-safe Rust decoders see uploaded bytes; libwebp
  (C) sees only raw pixels; its decoder is banned in `clippy.toml` and a test.
- **Attacker: decompression bombs and memory exhaustion.** Header checks before decoding,
  `image::Limits`, the JPEG SOF walk for progressive JPEGs, 2 image permits (peak about
  160 MiB each), 4 multipart permits, `mem_limit` 640 MiB.
- **Attacker: privacy leaks through metadata.** Re-encoding drops everything; originals are
  never stored.
- **Attacker: active content.** `image/webp`, `nosniff`, a sandbox CSP per response; admin
  views use their own routes.
- **Attacker: disk filling.** Quotas; spam rejection deletes images at once.

New dependencies:

- `image` (png, jpeg, webp decoders only; no default features): memory-safe decoding.
- `webp` (no default features): libwebp encoder bindings; brings `libwebp-sys` with a vendored
  C build script, added to `allow-build-scripts` deliberately.

## Impact

Migration 9 (`screenshots`). Multipart body class, image module, the `encode_webp` module,
admin screenshot routes, memory measurement per format (§12).

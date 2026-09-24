# Design

## Context

Design §6 Screenshots specifies the pipeline step by step; this change implements it as
written. Builds on `report-submission` (check order, insert) and `moderation` (views,
spam rejection).

## Decisions

**D1. Schema (`migrations/0009_screenshots.sql`).** `screenshots(id BLOB PRIMARY KEY (16
random bytes), report_id → reports CASCADE, width, height, bytes BLOB)` with `bytes` the last
column; `public_screenshots` view joined through `public_reports`.

**D2. Pipeline.** Exactly §6 steps 1–9: magic bytes; `ImageReader::with_guessed_format()
.into_decoder()` for dimensions and colour type; the JPEG SOF marker walk (~20 lines) for the
progressive charge; `image::Limits` (16 384 px, `max_alloc` 96 MiB); first frame;
`thumbnail(2560, 2560)` only when larger; `into_rgba8()`; `webp::Encoder::from_rgba(…)
.encode_simple(false, 80.0)`. The image semaphore (2, `try_acquire`) is held from decode
through encode in `spawn_blocking`.

**D3. libwebp isolation.** All `webp` use in `encode_webp.rs`; `clippy.toml`
`disallowed-types` `webp::Decoder`, `webp::AnimDecoder`, `webp::BitstreamFeatures` and
`disallowed-methods` `webp::Encoder::encode`, `encode_lossless`; the source scan fails on any
`libwebp_sys` path.

**D4. Multipart.** The route's own body class (25 MiB, 120 s, multipart accepted); axum
`multipart` feature (brings `multer`); every field read with `field.chunk()` and running
counters; the 4-permit semaphore.

**D5. Memory.** Peak per job measured per format and recorded (§12); the §3 budget (522 MiB
peak under `mem_limit: 640m`) is re-checked with those numbers.

## Risks / Trade-offs

- [libwebp is C] → Encoder only on raw pixels; advisories via `cargo deny`.
- [Quota overshoot under concurrency] → Bounded (4 × 3 files), documented.

## Migration Plan

Migration 9. Rollback before a release: revert.

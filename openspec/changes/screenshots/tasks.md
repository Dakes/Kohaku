# Tasks

- [ ] 1.1 Add `migrations/0009_screenshots.sql` (D1); verify the §12 migration test.
- [ ] 1.2 Add `image` and `webp` with dependency records, `deny.toml` build-script entry and
  the libwebp bans (D3); verify clippy rejects a planted `webp::Decoder` and the scan a
  `libwebp_sys` path.
- [ ] 1.3 Add header checks and the JPEG SOF walk (D2; screenshots: Image checks and
  re-encoding); verify "Attacker's decompression bomb" with checked-in fixtures.
- [ ] 1.4 Add decoding, downscaling and encoding with the image semaphore; verify metadata is
  gone, no upscaling, 503 with `Retry-After` when busy, and the §12 cases: 422 before decode
  for an 8000×5000 RGBA16 PNG, a 4000×4000 RGBA16 PNG and a 24 MP progressive 4:4:4 JPEG;
  success for a 24 MP RGBA8 PNG, a 24 MP baseline JPEG, a 10 MP progressive JPEG and the
  largest allowed lossy WebP with alpha; a 640×480 PNG keeps its size. Measure peak memory
  per format in a child process via `VmHWM` (≤ 160 MiB per job, ≤ 320 MiB for two) (D5).
- [ ] 1.5 Add the multipart route and permits (D4; screenshots: Uploads); verify "Attacker
  front-loads a file" and atomic storage with the report.
- [ ] 1.6 Add the quota settings and checks (configuration: Settings and defaults;
  screenshots: Quotas); verify "Screenshot quota values", "Project over quota" and the
  `/admin` notice.
- [ ] 1.7 Add public and admin serving, deletion and spam deletion (screenshots: Serving
  screenshots; moderation: Visibility); verify "Attacker guesses screenshot URLs",
  "Screenshot opened directly" and `screenshot.delete`.
- [ ] 1.7a Add cargo-fuzz targets for the image pipeline and multipart parsing (design §12),
  run manually on nightly outside CI; record a 10-minute run each in the PR.
- [ ] 1.8 README: enabling screenshots, quotas and their settings, `.env.example` entries
  (commented, with defaults).
- [ ] 1.9 Final check: the before-finishing commands and the smoke test with an upload.

# Spec Delta

## Purpose

Screenshots on reports of projects that enable them: how uploads are bounded, checked,
decoded in memory-safe code and re-encoded to lossy WebP, where they are stored and quotas,
and how they are served without ever becoming active content (design §6 Screenshots).

## ADDED Requirements

### Requirement: Uploads

Only the HTML form of a project with `screenshots_enabled` SHALL accept screenshots, as a
`multipart/form-data` body of at most 26 214 400 bytes (25 MiB) with a 120-second deadline
(http-security declared deviations); the JSON API never accepts them. The form SHALL put the
text fields before the files; they SHALL be checked (report-submission's order, the pending
caps and the quota included) before the first file byte is read, and a file part arriving
before them SHALL get 400. A request SHALL have at most 3 files of at most 8 MiB each, at most
12 parts and at most 64 KiB of text fields. At most 4 multipart submissions SHALL be read at
once (503 otherwise). A report's screenshots SHALL be stored in the same transaction as the
report, or not at all.

#### Scenario: Attacker front-loads a file
- **WHEN** an attacker sends a multipart body whose first part is a 25 MiB file, and another with 4 files, and another with a 9 MiB file
- **THEN** the first gets 400 before any file byte is buffered, the others get 422, and nothing is stored

### Requirement: Image checks and re-encoding

Each file SHALL be accepted only if its magic bytes are PNG, JPEG or WebP and its header
shows each side at most 16 384 pixels, at most 24 megapixels and a decode charge (pixels ×
bytes per pixel, plus for progressive JPEG the coefficient planes) of at most 96 MiB;
otherwise 422 before decoding. Decoding SHALL use only memory-safe Rust decoders with those
limits and the first frame only. Images larger than 2560 pixels on the longest side SHALL be
scaled down to 2560; smaller ones are never scaled up. The result SHALL be encoded as lossy
WebP at quality 80 by the libwebp encoder, which never receives the uploaded bytes; Kohaku
SHALL NOT call any libwebp decoder, keep the original file or any of its metadata. At most 2
images SHALL be processed at once; a submission finding both busy SHALL get 503 with
`Retry-After`.

#### Scenario: Attacker's decompression bomb
- **WHEN** an attacker uploads a 1 KiB PNG declaring 16 384 × 16 384 pixels, a progressive JPEG of 12 megapixels, a GIF renamed `.png`, and a PNG with an EXIF GPS block
- **THEN** the first three get 422 without decoding, and the fourth is stored as WebP without any of its metadata

### Requirement: Quotas

The stored WebP bytes SHALL be limited per project by `KOHAKU_SCREENSHOT_QUOTA_PROJECT_MIB`
and per instance by `KOHAKU_SCREENSHOT_QUOTA_INSTANCE_MIB` (configuration). Above either, the
form SHALL accept text only and say so, and `/admin` SHALL show the admin which quota is full.
The check runs before files are read; concurrent uploads MAY exceed a quota by at most 4
submissions of 3 files.

#### Scenario: Project over quota
- **WHEN** a project's screenshots total more than its quota and a reporter opens the form
- **THEN** the form has no file input and a posted file part gets 422

### Requirement: Serving screenshots

A public report's screenshot SHALL be served at `{base}/r/{n}/s/{id}` (a random 128-bit id)
only through the public views, with `Content-Type: image/webp`, `X-Content-Type-Options:
nosniff`, `Cache-Control: no-cache` and its own `Content-Security-Policy: default-src 'none';
frame-ancestors 'none'; sandbox`; any other id, report or project SHALL get the public 404.
Maintainers SHALL view a pending report's screenshots only at
`/admin/p/{slug}/r/{n}/s/{id}` and SHALL be able to delete a screenshot (audited
`screenshot.delete`). Rejecting a report as spam SHALL delete its screenshots in the same
transaction. The public detail page and API detail SHALL list the screenshot URLs.

#### Scenario: Attacker guesses screenshot URLs
- **WHEN** an attacker requests the public URL of a pending report's screenshot, a screenshot id under another report number, and another project's screenshot id on this project's host
- **THEN** each gets the public 404

#### Scenario: Screenshot opened directly
- **WHEN** a browser navigates to a screenshot URL
- **THEN** it is shown as an image with the sandbox CSP, and no script, style or navigation from it can run

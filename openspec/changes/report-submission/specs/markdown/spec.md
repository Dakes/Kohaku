# Spec Delta

## Purpose

Strict Markdown for report bodies and maintainer notes: what is accepted, how it is rendered
to HTML, and why nothing a reporter writes can run script, load a resource or break the page
(design §6 Markdown).

## ADDED Requirements

### Requirement: Accepted Markdown

Kohaku SHALL accept CommonMark with tables and strikethrough. Raw HTML (blocks and inline)
SHALL be rendered as literal text, images SHALL be dropped (their alt text kept as text), and
a link SHALL stay a link only when its destination is an absolute URL with scheme `http`,
`https` or `mailto`; any other link becomes its text. A document nesting block quotes, lists,
list items or tables deeper than 16 levels, or rendering to more than 262 144 bytes of HTML,
SHALL be rejected (422) when submitted.

#### Scenario: Attacker nests or amplifies
- **WHEN** an attacker submits 17 nested block quotes, and a 20 KiB body of reference-style links that would render past 256 KiB
- **THEN** both get 422 and nothing is stored

### Requirement: Rendering is sanitized

Rendered HTML SHALL pass an allow-list sanitizer after rendering: only the elements the
accepted Markdown produces, `href` as the only attribute of `a`, no `style` or event
attribute anywhere, link schemes `http`, `https` and `mailto` only, no relative URLs, and
every link with `rel="nofollow noopener noreferrer ugc"`. Rendering SHALL happen each time a
body is shown, never cached, so a sanitizer fix applies to old reports. Where a pending
report is shown to a maintainer, links SHALL be inert text showing their URL.

#### Scenario: Attacker's XSS corpus
- **WHEN** bodies contain `<script>`, `<img src=x onerror=…>`, `[a](javascript:alert(1))`, `[a](JaVaScRiPt:…)`, `[a](//evil.example)`, `[a](data:text/html,…)`, `![x](https://evil.example/p.png)`, `[a](tel:1)`, `<a href="https://x" style="…">`, an autolink `<javascript:alert(1)>` and entity-encoded variants
- **THEN** the rendered HTML contains no script, event attribute, `style`, image, or link with a scheme other than http, https or mailto, and every remaining link has the fixed `rel`

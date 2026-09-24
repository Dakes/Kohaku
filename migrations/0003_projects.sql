-- Schema 3 (change projects): projects, their switches and optional custom domains.

-- AUTOINCREMENT: a deleted project's id is never reused. Lengths count Unicode scalar
-- values except privacy_notice, which is capped in bytes; the application checks the
-- character rules.
CREATE TABLE projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    slug TEXT NOT NULL UNIQUE CHECK (
        length(slug) BETWEEN 1 AND 40 AND slug GLOB '[a-z0-9]*'
        AND NOT slug GLOB '*[^a-z0-9-]*'),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    public_host TEXT UNIQUE CHECK (
        public_host IS NULL OR (length(public_host) BETWEEN 3 AND 253
            AND public_host GLOB '*.*' AND NOT public_host GLOB '*[^a-z0-9.-]*')),
    screenshots_enabled INTEGER NOT NULL DEFAULT 0 CHECK (screenshots_enabled IN (0, 1)),
    features_enabled INTEGER NOT NULL DEFAULT 0 CHECK (features_enabled IN (0, 1)),
    require_email INTEGER NOT NULL DEFAULT 0 CHECK (require_email IN (0, 1)),
    plus_one_enabled INTEGER NOT NULL DEFAULT 1 CHECK (plus_one_enabled IN (0, 1)),
    privacy_notice TEXT CHECK (
        privacy_notice IS NULL OR length(CAST(privacy_notice AS BLOB)) BETWEEN 1 AND 1024),
    security_contact TEXT CHECK (
        security_contact IS NULL OR length(security_contact) BETWEEN 1 AND 254),
    next_number INTEGER NOT NULL DEFAULT 1 CHECK (next_number >= 1),
    created_at INTEGER NOT NULL
) STRICT;

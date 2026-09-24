-- Schema 2 (change admin-auth): accounts, sessions, device cookies, recovery codes and
-- single-use tokens; the outbox gains its account recipient.

-- Emails are stored normalized (trimmed, ASCII-lowercased). password_hash is NULL until
-- a password is set; totp_nonce is NULL while TOTP is not enrolled.
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE CHECK (length(email) <= 254),
    password_hash TEXT,
    role TEXT NOT NULL CHECK (role IN ('admin', 'maintainer')),
    disabled INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1)),
    totp_nonce BLOB CHECK (length(totp_nonce) = 16),
    totp_last_step INTEGER,
    failed_logins INTEGER NOT NULL DEFAULT 0 CHECK (failed_logins >= 0),
    locked_until INTEGER,
    device_failures INTEGER NOT NULL DEFAULT 0 CHECK (device_failures >= 0),
    device_window_start INTEGER,
    lockout_mailed_at INTEGER,
    created_at INTEGER NOT NULL
) STRICT;

-- At most one admin, enforced by the database itself.
CREATE UNIQUE INDEX users_one_admin ON users (role) WHERE role = 'admin';

-- Tokens and CSRF values as SHA-256 or random bytes only; never the cookie value.
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    csrf_token BLOB NOT NULL CHECK (length(csrf_token) = 32),
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    pending_totp_nonce BLOB CHECK (length(pending_totp_nonce) = 16)
) STRICT;

CREATE INDEX sessions_user ON sessions (user_id);

CREATE TABLE known_devices (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX known_devices_user ON known_devices (user_id);

CREATE TABLE recovery_codes (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code_hash BLOB NOT NULL CHECK (length(code_hash) = 32),
    PRIMARY KEY (user_id, code_hash)
) STRICT;

-- Reset, setup and invite tokens. Invites carry an email instead of an account; ids
-- appear in audit entries, so they are never reused.
CREATE TABLE tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    purpose TEXT NOT NULL CHECK (purpose IN ('reset', 'setup', 'invite')),
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    user_id INTEGER REFERENCES users (id) ON DELETE CASCADE,
    email TEXT CHECK (length(email) <= 254),
    totp_nonce BLOB CHECK (length(totp_nonce) = 16),
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    CHECK ((user_id IS NULL) = (purpose = 'invite')),
    CHECK ((email IS NULL) <> (purpose = 'invite'))
) STRICT;

CREATE INDEX tokens_user ON tokens (user_id);

-- The outbox with its account recipient's foreign key (change foundation D31.4).
CREATE TABLE outbox_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind GLOB '[a-z]*' AND NOT kind GLOB '*[^a-z_]*'
        AND length(kind) <= 64),
    user_id INTEGER REFERENCES users (id) ON DELETE CASCADE,
    address TEXT CHECK (length(address) <= 254),
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    priority TEXT NOT NULL CHECK (priority IN ('security', 'normal')),
    token INTEGER NOT NULL CHECK (token IN (0, 1)),
    placeholder INTEGER NOT NULL CHECK (placeholder IN (0, 1)),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at INTEGER NOT NULL,
    queued_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    outcome TEXT CHECK (outcome IN ('sent', 'given_up')),
    CHECK ((user_id IS NULL) <> (address IS NULL))
) STRICT;

INSERT INTO outbox_new (id, kind, user_id, address, subject, body, priority, token,
    placeholder, attempts, next_attempt_at, queued_at, expires_at, outcome)
SELECT id, kind, user_id, address, subject, body, priority, token,
    placeholder, attempts, next_attempt_at, queued_at, expires_at, outcome
FROM outbox;

-- Carry the id sequence over, so ids of already deleted rows stay unused.
DELETE FROM sqlite_sequence WHERE name = 'outbox_new';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'outbox_new', seq FROM sqlite_sequence WHERE name = 'outbox';

DROP TABLE outbox;
ALTER TABLE outbox_new RENAME TO outbox;

CREATE INDEX outbox_due ON outbox (next_attempt_at) WHERE outcome IS NULL;
CREATE INDEX outbox_user ON outbox (user_id);

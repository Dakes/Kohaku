-- Schema 1 (change foundation). Tables reference only earlier migrations; later
-- changes add columns and tables in their own migration.

-- One row: the keycheck of the instance secret that made this database.
CREATE TABLE meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    keycheck BLOB NOT NULL CHECK (length(keycheck) = 32),
    created_at INTEGER NOT NULL
) STRICT;

-- Outbound mail (mail-outbox). An account recipient (user_id) arrives with users; its
-- foreign key then too. Row ids are never reused: outcomes are written by id.
CREATE TABLE outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind GLOB '[a-z]*' AND NOT kind GLOB '*[^a-z_]*'
        AND length(kind) <= 64),
    user_id INTEGER,
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

CREATE INDEX outbox_due ON outbox (next_attempt_at) WHERE outcome IS NULL;

-- Append-only audit trail (audit-log): ids only, no foreign keys, so entries outlive
-- actors and targets. Columns are frozen: restore writes into every schema version.
CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_label TEXT NOT NULL CHECK (actor_label IN ('cli', 'admin', 'maintainer')),
    actor_id INTEGER CHECK ((actor_id IS NULL) = (actor_label = 'cli')),
    action TEXT NOT NULL CHECK (
        length(action) <= 64 AND action GLOB '[a-z]*' AND NOT action GLOB '*[^a-z0-9_.]*'
        AND NOT action GLOB '*.[^a-z]*' AND NOT action GLOB '*.'),
    target_type TEXT NOT NULL CHECK (
        length(target_type) <= 64 AND target_type GLOB '[a-z]*'
        AND NOT target_type GLOB '*[^a-z0-9_.]*'
        AND NOT target_type GLOB '*.[^a-z]*' AND NOT target_type GLOB '*.'),
    target_id INTEGER CHECK ((target_id IS NULL) = (target_type = 'instance')),
    time INTEGER NOT NULL
) STRICT;

CREATE TRIGGER audit_log_append_only BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit entries are never modified');
END;

-- Only retention deletes, and only entries more than 365 days old.
CREATE TRIGGER audit_log_keeps_a_year BEFORE DELETE ON audit_log
WHEN OLD.time >= unixepoch() - 365 * 86400
BEGIN
    SELECT RAISE(ABORT, 'audit entries are kept for 365 days');
END;

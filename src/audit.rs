//! The audit log: ids only, recorded in the caller's transaction (audit-log; change
//! foundation D19). No parameter takes a string.

use rusqlite::{Transaction, params};

/// Who acted. Users arrive with `admin-auth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Cli,
}

impl Actor {
    fn columns(self) -> (&'static str, Option<i64>) {
        match self {
            Actor::Cli => ("cli", None),
        }
    }
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    InstanceRestore,
}

impl Action {
    pub const ALL: &'static [Action] = &[Action::InstanceRestore];

    pub fn identifier(self) -> &'static str {
        match self {
            Action::InstanceRestore => "instance.restore",
        }
    }
}

/// What it happened to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The whole instance; no id.
    Instance,
}

impl Target {
    pub const TYPES: &'static [&'static str] = &["instance"];

    fn columns(self) -> (&'static str, Option<i64>) {
        match self {
            Target::Instance => ("instance", None),
        }
    }
}

/// Records one entry in `tx`, so it commits or rolls back with the change it records.
/// A refused entry fails the caller's transaction.
pub fn audit(
    tx: &Transaction<'_>,
    actor: Actor,
    action: Action,
    target: Target,
    now: i64,
) -> rusqlite::Result<()> {
    let (label, actor_id) = actor.columns();
    let (target_type, target_id) = target.columns();
    tx.execute(
        "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            label,
            actor_id,
            action.identifier(),
            target_type,
            target_id,
            now
        ],
    )
    .map(drop)
}

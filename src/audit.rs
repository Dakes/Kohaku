//! The audit log: ids only, recorded in the caller's transaction (audit-log; change
//! foundation D19). No parameter takes a string.

use rusqlite::{Transaction, params};

use crate::auth::Role;

/// Who acted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Cli,
    /// A signed-in account, or one using a token of its own (a reset link).
    User {
        id: i64,
        role: Role,
    },
}

impl Actor {
    fn columns(self) -> (&'static str, Option<i64>) {
        match self {
            Actor::Cli => ("cli", None),
            Actor::User { id, role } => (role.as_str(), Some(id)),
        }
    }
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    InstanceRestore,
    UserUnlock,
    UserResetLink,
    UserPasswordChange,
    UserPasswordReset,
    UserTotpEnable,
    UserTotpDisable,
    UserRecoveryCodes,
    UserSessionsEnd,
    ProjectCreate,
    ProjectUpdate,
    ProjectDelete,
}

impl Action {
    pub const ALL: &'static [Action] = &[
        Action::InstanceRestore,
        Action::UserUnlock,
        Action::UserResetLink,
        Action::UserPasswordChange,
        Action::UserPasswordReset,
        Action::UserTotpEnable,
        Action::UserTotpDisable,
        Action::UserRecoveryCodes,
        Action::UserSessionsEnd,
        Action::ProjectCreate,
        Action::ProjectUpdate,
        Action::ProjectDelete,
    ];

    pub fn identifier(self) -> &'static str {
        match self {
            Action::InstanceRestore => "instance.restore",
            Action::UserUnlock => "user.unlock",
            Action::UserResetLink => "user.reset_link",
            Action::UserPasswordChange => "user.password_change",
            Action::UserPasswordReset => "user.password_reset",
            Action::UserTotpEnable => "user.totp_enable",
            Action::UserTotpDisable => "user.totp_disable",
            Action::UserRecoveryCodes => "user.recovery_codes",
            Action::UserSessionsEnd => "user.sessions_end",
            Action::ProjectCreate => "project.create",
            Action::ProjectUpdate => "project.update",
            Action::ProjectDelete => "project.delete",
        }
    }
}

/// What it happened to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The whole instance; no id.
    Instance,
    User(i64),
    Project(i64),
}

impl Target {
    pub const TYPES: &'static [&'static str] = &["instance", "user", "project"];

    fn columns(self) -> (&'static str, Option<i64>) {
        match self {
            Target::Instance => ("instance", None),
            Target::User(id) => ("user", Some(id)),
            Target::Project(id) => ("project", Some(id)),
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

//! The account mails: fixed wording plus the main-host origin, a UTC time and, for the
//! reset mail, its link (admin-auth: Account mail content; change admin-auth D10).

use rusqlite::Transaction;

use crate::mail::outbox::{MailKind, Priority};
use crate::time::format_utc_minute;

/// Lifetime of a reset link, and of the mail carrying it.
pub const RESET_LIFETIME: i64 = 60 * 60;

/// Security mail is given up after the outbox's 24 hours.
const SECURITY_LIFETIME: i64 = 24 * 60 * 60;

pub static RESET_LINK: MailKind = MailKind {
    name: "account_reset_link",
    priority: Priority::Normal,
    token: true,
    lifetime: RESET_LIFETIME,
    give_up: Some(expire_reset_token),
};

pub static LOCKOUT: MailKind = MailKind {
    name: "account_lockout",
    priority: Priority::Security,
    token: false,
    lifetime: SECURITY_LIFETIME,
    give_up: None,
};

pub static PASSWORD_CHANGED: MailKind = MailKind {
    name: "account_password_changed",
    priority: Priority::Security,
    token: false,
    lifetime: SECURITY_LIFETIME,
    give_up: None,
};

/// A given-up reset mail's link stops working: an account has at most one unused
/// reset token, the one the mail carries.
fn expire_reset_token(tx: &Transaction<'_>, outbox_id: i64) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE tokens SET used_at = unixepoch()
         WHERE purpose = 'reset' AND used_at IS NULL
           AND user_id = (SELECT user_id FROM outbox WHERE id = ?1)",
        [outbox_id],
    )
    .map(drop)
}

/// A mail's subject and body.
pub struct Text {
    pub subject: &'static str,
    pub body: String,
}

pub fn reset_link(origin: &str, now: i64, link: &str) -> Text {
    Text {
        subject: "Reset your Kohaku password",
        body: format!(
            "Someone asked at {} to reset the password of your Kohaku account at {origin}.\n\
             \n\
             To choose a new password, open this link within one hour. It works once:\n\
             \n\
             {link}\n\
             \n\
             If you did not ask for this, ignore this mail: your password stays unchanged.\n",
            format_utc_minute(now)
        ),
    }
}

pub fn lockout(origin: &str, now: i64) -> Text {
    Text {
        subject: "Kohaku sign-in locked for 15 minutes",
        body: format!(
            "Your Kohaku account at {origin} was locked for 15 minutes at {} after 10 failed\n\
             sign-ins from browsers that have not signed in to it before.\n\
             \n\
             Browsers you signed in with before can still sign in. If the attempts were not\n\
             yours, someone may be guessing your password. You get this mail at most once a day.\n",
            format_utc_minute(now)
        ),
    }
}

pub fn password_changed(origin: &str, now: i64) -> Text {
    Text {
        subject: "Your Kohaku password was changed",
        body: format!(
            "The password of your Kohaku account at {origin} was changed at {}.\n\
             Every session of the account was signed out.\n\
             \n\
             If you did not change it, tell the operator of this Kohaku instance at once.\n",
            format_utc_minute(now)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_hold_only_fixed_text_origin_time_and_link() {
        let origin = "https://kohaku.example.org";
        let now = 1_800_000_000;
        let reset = reset_link(origin, now, "https://kohaku.example.org/admin/reset/T");
        assert!(reset.body.contains("2027-01-15 08:00 UTC"));
        assert!(
            reset
                .body
                .contains("\n\nhttps://kohaku.example.org/admin/reset/T\n\n")
        );
        for text in [reset, lockout(origin, now), password_changed(origin, now)] {
            assert!(text.body.contains(origin));
            assert!(text.body.is_ascii());
            assert!(!text.subject.contains(['\r', '\n']));
        }
    }
}

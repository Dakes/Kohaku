# Proposal

## Why

Reports are private until someone approves them, so the admin and maintainers must learn
that reports are waiting, without a spam wave becoming hundreds of mails (design §9, §15
item 7). With this change the tracker is usable.

## What Changes

- "Reports waiting" mail per project to the admin and granted maintainers, coalesced to at
  most one per recipient and project per 10 minutes, with the current pending count.
- Per-project opt-out on the account page; revoking a grant or opting out deletes unsent mail.
- Outbox rows gain a project reference so that project deletion removes them.

## Capabilities

### New Capabilities

- `notifications`: new-report mail and opt-outs.

### Modified Capabilities

- `audit-log`: `user.notify`.
- `users-and-invites`: revoking a grant deletes unsent mail for that project.

## Non-goals

- Mail about status changes to reporters (`reporter-verification`), digests, mail about
  approved reports, chat or webhook integrations.

## Security considerations

- **Attacker: mail-bombs maintainers with submissions.** Coalescing bounds mail to one per
  recipient and project per 10 minutes; the submission limits still apply.
- **Attacker: phishing through report text.** The mail carries no user-supplied text, only
  the admin-set project name, a count and a main-host link.

No new dependencies.

## Impact

Migration 7 (`notify_optouts`, `outbox.project_id`). Account page section, one mail kind.

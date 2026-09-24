# Spec Delta

## Purpose

Mail to the admin and maintainers when reports wait for review: who receives it, how a
spam wave becomes one mail, what the mail may contain, and per-project opt-out (design §9
Maintainers).

## ADDED Requirements

### Requirement: New-report mail

When a submission creates a pending report, Kohaku SHALL, in the same transaction, make sure
each recipient of that project has one unsent "reports waiting" mail for it. Recipients are
the admin and the maintainers granted the project, each only while the account is enabled,
has a password, and has not opted out for that project. If the recipient already has an
unsent one for the project, Kohaku SHALL update its count instead of adding a mail; otherwise
it SHALL queue one due no earlier than 10 minutes after the last one sent to that recipient
for that project. So each recipient gets at most one such mail per project per 10 minutes.

The mail SHALL contain only fixed wording, the project's name, the number of pending reports
in the project when the mail was last updated, and the link to the project's pending list on
the main host; no title, body, reporter data or other user-supplied text. It SHALL have normal
priority and expire 24 hours after queuing.

#### Scenario: Spam wave becomes one mail
- **WHEN** 500 reports arrive in project demo within 5 minutes
- **THEN** each recipient has at most one mail for demo sent in any 10 minutes, and the next one states the current number of pending reports

#### Scenario: Attacker text never reaches a mailbox
- **WHEN** an attacker submits a report titled `Your password expired, log in at https://evil.example`
- **THEN** no queued mail contains any part of the title or body

### Requirement: Opting out per project

On the account page every user SHALL see the projects they receive mail for and switch each
on or off (on for new grants and for the admin's projects). Switching one off, and the admin
revoking a grant, SHALL delete that user's unsent mail for that project in the same
transaction. Each switch SHALL be audited as `user.notify` (target type `user`).

#### Scenario: Opt-out while a mail waits
- **WHEN** a maintainer with an unsent "reports waiting" mail for demo switches demo off
- **THEN** the mail is deleted unsent and no further one is queued for demo until switched on again

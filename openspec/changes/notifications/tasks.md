# Tasks

- [ ] 1.1 Add `migrations/0007_notifications.sql` (D1); verify the §12 migration test and that
  deleting a project deletes its unsent mail.
- [ ] 1.2 Add the mail kind and coalescing in the submission transaction (D2; notifications:
  New-report mail); verify "Spam wave becomes one mail" with injected time and "Attacker text
  never reaches a mailbox".
- [ ] 1.3 Add the opt-out section on the account page and grant-revoke cleanup
  (notifications: Opting out per project; users-and-invites: Grants and project access);
  verify "Opt-out while a mail waits", grant revocation and `user.notify`.
- [ ] 1.4 README: which mail the admin and maintainers get and how to switch it off.
- [ ] 1.5 Final check: the before-finishing commands; a dev-build run shows the printed mail.

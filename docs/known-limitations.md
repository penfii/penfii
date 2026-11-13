# Known limitations

Penfii is at v0.1.0, an early release. This list says what is missing or not yet safe to rely on. It is kept short on purpose; if you find something that belongs here, open an issue.

## Stability

- The HTTP API, the database schema and the settings can change between 0.x releases, and there is no upgrade guarantee.
- Backups, upgrades and monitoring are your responsibility. A database restore check is in [Self-hosting](self-hosting.md#restore) (`docker/scripts/restore-check.sh`). It does not start the application on the restored data, and there is no point-in-time recovery. There is still no procedure for moving between releases.
- Object storage is assumed not to use S3 Versioning. Deleting a file removes the current object after the database commit; older versions in a versioned bucket are left behind.

## Invitations and e-mail

- An invitation is a link that the inviter copies; Penfii mails it as well only when SMTP is configured (`PENFII_SMTP_*`, see [Configuration](configuration.md#e-mail)). The link can be shown only when the invitation is made or made again, because only its hash is stored.
- E-mail is sent once, while the invitation is saved, with no queue and no automatic reminder or retry. A failed mail is shown in the invitation list and "New link" tries again. The server certificate of the SMTP server is always checked against the system authorities; there is no way to turn that off.
- A person can reset their own password by e-mail only while the server sends e-mail (see [Configuration](configuration.md#e-mail)); an account that has no password (it only signs in through a provider) cannot be given one this way. Without e-mail, or for such an account, an operator uses `admin-cli user-set-password`.
- Reset requests are answered the same for every address, so a person who mistypes theirs is not told; the mail simply does not come. A reset mail for one account can be asked for as often as the reset counters allow (5 from one address in 15 minutes by default).
- There is no public sign-up. Workspace creation through the API is open only while the instance has no workspace.

## Search

- Content search (messages, tickets, docs, files) requires OpenSearch with `analysis-nori`. There is no PostgreSQL fallback; without OpenSearch it answers `503`. Finding people, teams, projects and channels does not need it.
- There is no English stemming: English text is matched as lower-cased tokens and n-grams.
- Files are found by name, not by what is inside them.

## Browsers

What has been checked (2026-11-13, the whole Playwright suite in `client/apps/e2e` against a running stack):

| Browser | How | Result |
|---|---|---|
| Chromium 158 (Linux) | Playwright | all scenarios pass |
| Firefox (Playwright build, Linux) | Playwright | all scenarios pass |
| WebKit (Playwright build, Linux; the engine of Safari, not Safari itself) | Playwright in its Docker image | all scenarios pass; it logs a harmless warning that it does not know the `interactive-widget` viewport hint |
| Safari on macOS or iOS, Chrome on Android | not run | not verified |
| A real input method (Korean, Japanese, Chinese) on any OS | not run | not verified |

For input methods the page was checked only with the events a browser reports while composing (Enter with `isComposing`, and Safari's late Enter with `keyCode` 229) in all three engines: the command palette, the link box and the message box do not act on them. That is not the same as typing with a real IME; a person should try Korean input on macOS Safari, Windows and Android before a release.

## Docs and co-editing

- The server has a live co-editing socket for docs, but the web app does not use it: the editor saves explicitly, and a stale save is refused with a conflict notice. The web client has no co-editing code (it was removed; it had never been run against the socket in a browser), so using the socket again means writing a client for it. `yjs`, `y-prosemirror` and `y-protocols` stay in the client only as dev dependencies: the test that holds BlockNote's conversion to the Rust seeder's (`modules/docs/golden.test.ts`, shared with `server/crates/core/src/docs/yjs.rs`) uses them, and nothing that ships imports them.
- Co-editing runs on one core-api instance; a second instance answers co-editing sockets with `503`.

## Files and tickets

- Files attach to chat messages, docs, ticket bodies and ticket comments. Ticket attachments have no count or total-size limit of their own, only `PENFII_FILE_MAX_BYTES` per file.
- A ticket comment can be edited and deleted. Its files are chosen when it is posted: editing the comment does not change them, and deleting the comment removes them.
- Channel memberships (private and public) are cleared when a person is removed from a Workspace, so a person who joins again is in no channel. Memberships left by a removal made before this are cleared when the person comes back. The same holds for a Project: when a person stops taking part in it, their memberships of its channels go too and taking part again restores none; a membership an older version left for a person outside the Project grants nothing and is removed by the upgrade. A public channel can end up with no manager (its creator is its only manager and was removed or disabled); nothing recovers it, and with no manager nobody can delete other people's messages in it, and a workspace owner or admin or the Project's admin has no moderation power over it (that is for a later policy). Everything else in it works.
- Private channels: the manager and the admin of the Project run a channel's people and settings (the admin without reading it; see [Operations](operations.md)). A private channel without an active manager is recovered by a workspace owner or admin naming one in the web app. An admin of the Project who is not in a channel cannot read it, put themself in or make it public; they find it under the Project's settings. A webhook of a private channel can be seen and revoked by that admin, but only someone who may post issues or rotates one.
- Trashed tickets stay until a Project admin restores or purges them, unless `PENFII_TRASH_RETENTION_DAYS` is set to a positive number of days: the automatic purge is **off by default**. A trashed ticket that still has children or dependencies is never purged, by the sweep or by hand, until those are resolved; it stays in the trash and is shown as held.
- Archived tickets (a Project admin archives; the ticket leaves boards, lists and search, stays readable and becomes read-only) are hidden from pins, favorites and Later rather than shown there as archived. Archive and trash are independent: restoring from the trash keeps the archive, and a ticket that is also trashed must be restored before it can be unarchived. A ticket with an unfinished child cannot be archived.
- Archive, unarchive, trash, restore and purge are Project-admin powers that do not depend on the ticket's private channel, and they change nobody's access to it: a Project admin who is not in the channel can archive such a ticket (and a completed Sprint archives it too), while its detail and the Archived tickets list stay hidden from them. The trash list, which predates the archive, does not filter by channel and shows the key and title of every trashed ticket of the Project to its admin.
- A completed Sprint's finished work is archived immediately or at a time chosen when the Sprint is completed. The choice is made once, in the completion dialog; afterwards only the scheduled time can be changed or the schedule canceled. If the person who scheduled it is no longer a Project admin when it is due, the schedule is canceled instead of archiving on their behalf. A server that was down at the due time archives when it is back.
- Bulk archive and trash on the Board are sent in requests of at most 100 tickets. If a request fails part-way, the tickets not yet processed are reported as such and stay selected; there is no server-side all-or-nothing.

## Deployment

- Only the Docker Compose stack in `docker/` is tested end to end. Pointing the server at your own PostgreSQL, S3 store or OpenSearch is possible through environment variables but is not packaged or tested.
- Only MinIO has been used as the S3 store, and it must accept path-style requests.
- OpenSearch runs with its security plugin disabled and is not published; do not expose it.
- Live updates are per process: the notices that make open sockets look again (chat, ticket boards, Inbox, Space) travel in the memory of the core-api that handled the change. With more than one core-api instance, a person connected to another instance sees changes only when their client reloads or reconnects. Run one instance; see [Running more than one core-api](operations.md#running-more-than-one-core-api).
- The `/mcp` rate limit is per process.

## Client

- Development and browser checks have used Chromium only. Safari, Firefox, and checks with a real Korean input method on each operating system have not been done.
- Korean and English only. The first visit uses the browser's language (English when it asks for neither); a language chosen with the toggle is saved in `localStorage` and wins afterwards.
- First-load JavaScript (the entry script and its modulepreloads) is about 690 KB, 201 KB gzipped (measured 2026-11-13), plus the texts of the language in use (about 38 KB gzipped, fetched by the entry; the other language loads only when chosen). Pages, the message and ticket editors (tiptap/ProseMirror), the code highlighter (shiki) and the document editor load when used. The document editor chunk is still over 500 KB, which Vite warns about. `make check` runs `client/scripts/check-bundle-size.mjs`, which fails when the first load grows past its budget or holds one of those libraries again; the budgets are in that script.
- Only part of the API types in `client/packages/contracts` come from `openapi.json` (`src/generated.ts`, aliased in `src/index.ts`): the models whose schema lists every required field and closes its sets of values. The rest, among them Ticket, ProjectBoard, Workspace, Project, Channel, Conversation and Message, are written by hand because their schemas are looser than what the server sends (properties not listed as required, values typed as plain strings, a few fields missing). A test compares those with the schema of the same name, but cannot check one that has no schema; tightening the schemas and aliasing the model is the way to move one over.
- No desktop or installable mobile app, and no offline mode.
- Admin CLI: the password of `bootstrap`, `create-user` and `user-set-password` can be given with `--password-stdin` or a prompt; `--password <value>` still works but leaves the value in the shell history and the process list, so it warns.
- A Team-narrowed channel is open only to people in both the Project and the Team: its creator has to be in the Team too (a Project admin outside the Team is refused with `403`; there is no creator exception), and a person who leaves the Team loses their membership of it.

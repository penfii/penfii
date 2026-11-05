# Known limitations

Penfii is at v0.1.0, an early release. This list says what is missing or not yet safe to rely on. It is kept short on purpose; if you find something that belongs here, open an issue.

## Stability

- The HTTP API, the database schema and the settings can change between 0.x releases, and there is no upgrade guarantee.
- Backups, upgrades and monitoring are your responsibility. There is no tested procedure for restoring from a backup or for moving between releases yet.

## Invitations and e-mail

- Penfii does not send e-mail. An invitation produces a link token that is returned to the inviter only when the server runs with the development switch `PENFII_DEV_RETURN_INVITE_TOKEN=1`. How invitation links reach people is not decided yet.
- There is no password reset by e-mail; an operator uses `admin-cli user-set-password`.
- `admin-cli create-user` makes an account with a workspace of its own; it does not add the person to an existing workspace.
- There is no public sign-up. Workspace creation through the API is open only while the instance has no workspace.

## Search

- Content search (messages, tickets, docs, files) requires OpenSearch with `analysis-nori`. There is no PostgreSQL fallback; without OpenSearch it answers `503`. Finding people, teams, projects and channels does not need it.
- There is no English stemming: English text is matched as lower-cased tokens and n-grams.
- Files are found by name, not by what is inside them.

## Docs and co-editing

- The server has a live co-editing socket for docs, but the shipped web app does not use it: the editor saves explicitly, and a stale save is refused with a conflict notice. The client code for the socket exists (`client/packages/client/src/modules/docs/DocCollabEditor.tsx`) but no screen mounts it, and it has not been run against the socket in a browser.
- Co-editing runs on one core-api instance; a second instance answers co-editing sockets with `503`.

## Files and tickets

- Files attach to chat messages, docs, ticket bodies and ticket comments. Ticket attachments have no count or total-size limit of their own, only `PENFII_FILE_MAX_BYTES` per file.
- A ticket comment can be edited and deleted. Its files are chosen when it is posted: editing the comment does not change them, and deleting the comment removes them.
- Private channels: a manager can add people and change roles, and anyone can leave (see [Operations](operations.md)), but there is no way to remove another person. A channel whose only manager loses workspace access has no manager, and no screen lets an administrator recover it. The setting that lets any member add people (`allowMemberInvites`) has no screen or API to turn it on.
- Trashed tickets stay until someone restores or purges them; there is no automatic expiry.

## Deployment

- Only the Docker Compose stack in `docker/` is tested end to end. Pointing the server at your own PostgreSQL, S3 store or OpenSearch is possible through environment variables but is not packaged or tested.
- Only MinIO has been used as the S3 store, and it must accept path-style requests.
- OpenSearch runs with its security plugin disabled and is not published; do not expose it.
- The `/mcp` rate limit is per process.

## Client

- Development and browser checks have used Chromium only. Safari, Firefox, and checks with a real Korean input method on each operating system have not been done.
- Korean and English only. The first visit uses the browser's language (English when it asks for neither); a language chosen with the toggle is saved in `localStorage` and wins afterwards.
- The main JavaScript chunk is large (about 2.6 MB, 574 KB gzipped when last measured); routes are not split yet except the document editor.
- The API types in `client/packages/contracts` are written by hand. A test compares them with a copy of the server's `openapi.json`, but it cannot check types that have no schema of the same name.
- No desktop or installable mobile app, and no offline mode.

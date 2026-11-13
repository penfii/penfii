-- FEA-062: an invitation link is a product feature that does not need e-mail, and sending it by
-- e-mail is an optional delivery on top of it.
--
-- 1. A Workspace has at most one open invitation (not accepted, not revoked) per e-mail: inviting
--    the same address again re-issues that row (new token, new expiry) instead of adding another.
--    Open invitations that earlier versions allowed to pile up are reduced to the newest one; the
--    older ones are revoked, which is what a person would have done by hand (their links stop working).
-- 2. The last e-mail delivery of an invitation is recorded next to it. It is separate from the
--    invitation's own state: a failed or missing e-mail never changes whether the link works.
UPDATE penfii.workspace_invitations i
SET revoked_at = now()
WHERE i.accepted_at IS NULL
  AND i.revoked_at IS NULL
  AND EXISTS (
      SELECT 1 FROM penfii.workspace_invitations newer
      WHERE newer.workspace_id = i.workspace_id
        AND newer.email_key = i.email_key
        AND newer.accepted_at IS NULL
        AND newer.revoked_at IS NULL
        AND (newer.created_at, newer.id) > (i.created_at, i.id)
  );

CREATE UNIQUE INDEX workspace_invitations_open_uq
    ON penfii.workspace_invitations (workspace_id, email_key)
    WHERE accepted_at IS NULL AND revoked_at IS NULL;

ALTER TABLE penfii.workspace_invitations
    ADD COLUMN email_status text,
    ADD COLUMN email_attempted_at timestamp with time zone,
    ADD COLUMN email_error text,
    ADD CONSTRAINT workspace_invitations_email_status_chk
        CHECK (email_status IS NULL OR email_status IN ('NOT_CONFIGURED', 'SENT', 'FAILED'));

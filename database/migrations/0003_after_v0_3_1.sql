-- Every schema and data change after v0.3.1, as one file. It brings a v0.3.1 database (and a fresh one,
-- right after 0002) to the next release's schema, in one transaction: a failure leaves the v0.3.1
-- schema and data as they were.
--
-- Development builds between v0.3.1 and this file had the same changes split into 0003-0007
-- (email change, removed member access, auth attempt browser, mfa totp, removed member access
-- followup). `admin-cli migrate` turns that recorded history into this one version when, and only
-- when, all five were applied unchanged (server/crates/core/src/migrate.rs, `consolidate_split_0003`);
-- they are not applied again.
--
-- The sections keep their order: the tables first, then the data clean-up.

-- ===== email change =====

-- FEA-140: changing an account's e-mail address is confirmed from the new address before it takes
-- effect. The request holds the address that is waiting; the account keeps signing in and recovering
-- with its current address until the link mailed to the new one is used. A token is a secret that is
-- mailed once and stored only as its hash; it works once (`closed_at` is set when it is used or
-- replaced by a newer request) and only until `expires_at`.

CREATE TABLE penfii.email_change_requests (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES penfii.users(id) ON DELETE CASCADE,
    new_email text NOT NULL,
    new_email_key text NOT NULL,
    token_hash bytea NOT NULL UNIQUE,
    expires_at timestamp with time zone NOT NULL,
    closed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

-- At most one waiting request per account, whatever the order in which requests arrive.
CREATE UNIQUE INDEX email_change_requests_one_open_idx ON penfii.email_change_requests (user_id) WHERE closed_at IS NULL;
CREATE INDEX email_change_requests_expires_idx ON penfii.email_change_requests (expires_at);

-- ===== auth attempt browser =====

-- FEA-143: an external sign-in attempt belongs to the browser that started it. The start sets a short-lived
-- cookie whose random value is kept here as a hash; the callback only consumes an attempt whose cookie
-- it can show. An attempt started before this column existed has no hash and is not consumed (it lives
-- ten minutes at most; the person signs in again).

ALTER TABLE penfii.auth_attempts ADD COLUMN browser_hash bytea;

-- ===== mfa totp =====

-- FEA-150: optional TOTP two-step sign-in with single-use recovery codes.
--
-- mfa_totp: one row per account that started or finished setting up an authenticator app. A row with
--   enabled_at NULL is a setup that was not confirmed yet (it does not protect sign-in). The shared
--   secret is stored encrypted (AES-256-GCM, the account id as associated data); key_id names the key
--   (the first 8 bytes of its SHA-256) so a lost or changed key is reported, not guessed at.
--   last_step is the newest 30-second step already accepted: a code is good once.
-- mfa_recovery_codes: the codes of an enabled account as SHA-256 hashes; used_at marks a spent code.
-- mfa_instance_key: the key that encrypts the secrets unless PENFII_MFA_ENCRYPTION_KEY gives another
--   one, so enabling MFA needs no new setting. It lives in the database and so in its backups; see
--   docs/operations.md before relying on it to protect a stolen backup.

CREATE TABLE penfii.mfa_instance_key (
    id smallint PRIMARY KEY CHECK (id = 1),
    key bytea NOT NULL CHECK (length(key) = 32),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE penfii.mfa_totp (
    user_id uuid PRIMARY KEY REFERENCES penfii.users (id) ON DELETE CASCADE,
    key_id bytea NOT NULL,
    nonce bytea NOT NULL,
    secret_ciphertext bytea NOT NULL,
    enabled_at timestamptz,
    last_step bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE penfii.mfa_recovery_codes (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES penfii.users (id) ON DELETE CASCADE,
    code_hash bytea NOT NULL,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, code_hash)
);

-- ===== removed member access =====

-- FEA-149: a person who is removed from a Workspace loses the access that was explicitly given to
-- them there, so coming back (a new invitation, or the operator adding them) starts from a new
-- membership instead of restoring the old one. From this release the removal itself does it
-- (`membership::revoke_explicit_access`). This section does the same for the people removed by
-- earlier releases, whose rows were left behind:
--   * Team memberships of the Workspace's Teams,
--   * direct Project grants on the Workspace's Projects,
--   * Group memberships of the Workspace's Groups,
--   * invited participant rows of Doc pages of the Workspace, a MANAGER row too, even when no other
--     manager would remain (the page stays; a Workspace owner or admin names a manager for it),
--   * rows in `workspace_security_auditors` (read access to the audit log) for that Workspace,
--   * rows in `channel_content_auditors` (read access to a private channel's content) for the
--     channels of that Workspace.
-- It only touches a person whose membership of that Workspace is INACTIVE right now. A person who was
-- removed and has since been taken back keeps what they hold. Every statement is scoped by the
-- Workspace of the row, so accounts' rights in any other Workspace are untouched. It never deletes
-- records of what the person wrote or did (messages, Docs, Tickets, audit entries), and it leaves alone
-- the participant row on a page the person created (the author keeps their access to their own page)
-- and direct messages (`dm_members` is a relation, not a grant). Channel memberships were already
-- cleared on removal and are not touched.
-- One audit entry (`member.access_cleaned`, `via: migration 0003`) is added per person and Workspace
-- that lost rows; it also names the Projects where the person was a direct admin, which the list of
-- Projects without an admin reads to say who held the role.

WITH teams_gone AS (
    DELETE FROM penfii.team_members tm
    USING penfii.teams t, penfii.workspace_members wm
    WHERE tm.team_id = t.id
      AND wm.workspace_id = t.workspace_id AND wm.user_id = tm.user_id AND wm.state = 'INACTIVE'
    RETURNING t.workspace_id, tm.user_id, 'teams'::text AS kind, NULL::uuid AS admin_of
), projects_gone AS (
    DELETE FROM penfii.project_user_grants g
    USING penfii.projects p, penfii.workspace_members wm
    WHERE g.project_id = p.id
      AND wm.workspace_id = p.workspace_id AND wm.user_id = g.user_id AND wm.state = 'INACTIVE'
    RETURNING p.workspace_id, g.user_id, 'projects'::text AS kind,
              CASE WHEN g.role IN ('ADMIN', 'OWNER') THEN g.project_id END AS admin_of
), groups_gone AS (
    DELETE FROM penfii.group_members gm
    USING penfii.groups gr, penfii.workspace_members wm
    WHERE gm.group_id = gr.id
      AND wm.workspace_id = gr.workspace_id AND wm.user_id = gm.user_id AND wm.state = 'INACTIVE'
    RETURNING gr.workspace_id, gm.user_id, 'groups'::text AS kind, NULL::uuid AS admin_of
), docs_gone AS (
    DELETE FROM penfii.doc_page_participants pp
    USING penfii.doc_pages p, penfii.workspace_members wm
    WHERE pp.page_id = p.id
      AND wm.workspace_id = p.workspace_id AND wm.user_id = pp.user_id AND wm.state = 'INACTIVE'
      AND p.created_by <> pp.user_id
    RETURNING p.workspace_id, pp.user_id, 'docShares'::text AS kind, NULL::uuid AS admin_of
), security_gone AS (
    DELETE FROM penfii.workspace_security_auditors a
    USING penfii.workspace_members wm
    WHERE wm.workspace_id = a.workspace_id AND wm.user_id = a.user_id AND wm.state = 'INACTIVE'
    RETURNING a.workspace_id, a.user_id, 'securityAuditor'::text AS kind, NULL::uuid AS admin_of
), channel_auditors_gone AS (
    DELETE FROM penfii.channel_content_auditors a
    USING penfii.conversations c, penfii.workspace_members wm
    WHERE a.channel_id = c.id
      AND wm.workspace_id = c.workspace_id AND wm.user_id = a.user_id AND wm.state = 'INACTIVE'
    RETURNING c.workspace_id, a.user_id, 'channelAuditors'::text AS kind, NULL::uuid AS admin_of
), cleaned AS (
    SELECT * FROM teams_gone
    UNION ALL SELECT * FROM projects_gone
    UNION ALL SELECT * FROM groups_gone
    UNION ALL SELECT * FROM docs_gone
    UNION ALL SELECT * FROM security_gone
    UNION ALL SELECT * FROM channel_auditors_gone
)
INSERT INTO penfii.audit_entries
    (id, workspace_id, actor_user_id, action_code, outcome_code, target_kind, target_id, command_id, data)
SELECT gen_random_uuid(), c.workspace_id, NULL, 'member.access_cleaned', 'OK', 'USER', c.user_id, gen_random_uuid(),
       jsonb_build_object(
           'userId', c.user_id,
           'via', 'migration 0003',
           'directAdminOf', COALESCE(jsonb_agg(c.admin_of) FILTER (WHERE c.admin_of IS NOT NULL), '[]'::jsonb),
           'revoked', jsonb_build_object(
               'teams', count(*) FILTER (WHERE c.kind = 'teams'),
               'projects', count(*) FILTER (WHERE c.kind = 'projects'),
               'groups', count(*) FILTER (WHERE c.kind = 'groups'),
               'docShares', count(*) FILTER (WHERE c.kind = 'docShares'),
               'securityAuditor', count(*) FILTER (WHERE c.kind = 'securityAuditor'),
               'channelAuditors', count(*) FILTER (WHERE c.kind = 'channelAuditors')))
FROM cleaned c
GROUP BY c.workspace_id, c.user_id;

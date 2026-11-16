-- Every schema and data change after v0.1.0, as one file. v0.1.0 shipped 0001_initial.sql alone; this
-- file brings a v0.1.0 database (and a fresh one, right after 0001) to the v0.2.0 schema. It runs in one
-- transaction: a failure leaves the v0.1.0 schema and data as they were.
--
-- The sections keep their order: the data clean-ups run before the constraints that rely on them.

-- ===== inbox archived at =====

-- Inbox: when an item was archived. Written by the `archive` command, cleared when the item leaves
-- the archive. Items archived before this column existed have no value (it cannot be recovered);
-- the archive lists them by `created_at` instead.

ALTER TABLE penfii.inbox_items ADD COLUMN archived_at timestamp with time zone;

CREATE INDEX inbox_items_archived_idx ON penfii.inbox_items USING btree (recipient_id, workspace_id, (COALESCE(archived_at, created_at)) DESC, id DESC) WHERE (state = 'ARCHIVED'::text);

-- ===== push subscriptions =====

-- Web Push: one row per browser/device a person subscribed from. It belongs to the person, not to a
-- session, so signing out does not remove it; the push service answering 404/410 does, and so does
-- the person turning the device off. The VAPID private key is not stored here (server secret).

CREATE TABLE penfii.push_subscriptions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    endpoint text NOT NULL,
    p256dh text NOT NULL,
    auth text NOT NULL,
    user_agent text,
    label text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    last_success_at timestamp with time zone,
    last_failure_at timestamp with time zone,
    CONSTRAINT push_subscriptions_pkey PRIMARY KEY (id),
    CONSTRAINT push_subscriptions_endpoint_key UNIQUE (endpoint),
    CONSTRAINT push_subscriptions_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id) ON DELETE CASCADE,
    CONSTRAINT push_subscriptions_endpoint_len CHECK (char_length(endpoint) BETWEEN 1 AND 2048),
    CONSTRAINT push_subscriptions_keys_len CHECK (char_length(p256dh) <= 200 AND char_length(auth) <= 100),
    CONSTRAINT push_subscriptions_label_len CHECK (label IS NULL OR char_length(label) <= 100),
    CONSTRAINT push_subscriptions_agent_len CHECK (user_agent IS NULL OR char_length(user_agent) <= 400)
);

CREATE INDEX push_subscriptions_user_idx ON penfii.push_subscriptions USING btree (user_id);

-- ===== incoming webhooks =====

-- Incoming webhooks. An Integration Actor is a users row so it can author a message, and nothing else:
-- it cannot sign in, be invited, or join a workspace, channel, team, or direct message.

ALTER TABLE penfii.users
    ADD COLUMN kind text DEFAULT 'HUMAN' NOT NULL;

ALTER TABLE penfii.users
    ADD CONSTRAINT users_kind_chk CHECK ((kind = ANY (ARRAY['HUMAN'::text, 'INTEGRATION'::text])));

CREATE TABLE penfii.integration_actors (
    user_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    created_by uuid NOT NULL,
    disabled_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT integration_actors_pkey PRIMARY KEY (user_id),
    CONSTRAINT integration_actors_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id),
    CONSTRAINT integration_actors_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id),
    CONSTRAINT integration_actors_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id)
);

CREATE INDEX integration_actors_workspace_idx ON penfii.integration_actors USING btree (workspace_id);

CREATE TABLE penfii.webhooks (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    channel_id uuid NOT NULL,
    actor_user_id uuid NOT NULL,
    name text NOT NULL,
    auth_mode text NOT NULL,
    token_hash bytea,
    secret_ciphertext bytea,
    secret_nonce bytea,
    created_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    rotated_at timestamp with time zone,
    revoked_at timestamp with time zone,
    CONSTRAINT webhooks_pkey PRIMARY KEY (id),
    CONSTRAINT webhooks_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id),
    CONSTRAINT webhooks_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES penfii.channels(id),
    CONSTRAINT webhooks_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES penfii.integration_actors(user_id),
    CONSTRAINT webhooks_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id),
    CONSTRAINT webhooks_auth_mode_chk CHECK ((auth_mode = ANY (ARRAY['URL_SECRET'::text, 'SIGNATURE'::text]))),
    CONSTRAINT webhooks_name_len CHECK (((char_length(name) >= 1) AND (char_length(name) <= 200))),
    CONSTRAINT webhooks_credential_chk CHECK ((((auth_mode = 'URL_SECRET'::text) AND (token_hash IS NOT NULL) AND (secret_ciphertext IS NULL) AND (secret_nonce IS NULL)) OR ((auth_mode = 'SIGNATURE'::text) AND (token_hash IS NULL) AND (secret_ciphertext IS NOT NULL) AND (secret_nonce IS NOT NULL))))
);

CREATE UNIQUE INDEX webhooks_token_hash_uidx ON penfii.webhooks USING btree (token_hash) WHERE (token_hash IS NOT NULL);

CREATE INDEX webhooks_channel_idx ON penfii.webhooks USING btree (channel_id, created_at);

CREATE TABLE penfii.webhook_post_windows (
    webhook_id uuid NOT NULL,
    window_start timestamp with time zone DEFAULT now() NOT NULL,
    posts integer DEFAULT 0 NOT NULL,
    CONSTRAINT webhook_post_windows_pkey PRIMARY KEY (webhook_id),
    CONSTRAINT webhook_post_windows_webhook_id_fkey FOREIGN KEY (webhook_id) REFERENCES penfii.webhooks(id),
    CONSTRAINT webhook_post_windows_posts_chk CHECK ((posts >= 0))
);

CREATE FUNCTION penfii.reject_integration_membership() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM penfii.users u
        WHERE u.id = NEW.user_id AND u.kind = 'INTEGRATION'
    ) THEN
        RAISE EXCEPTION 'integration actors cannot be members'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER workspace_members_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.workspace_members
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER channel_members_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.channel_members
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER team_members_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.team_members
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER dm_members_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.dm_members
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER group_members_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.group_members
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER project_user_grants_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.project_user_grants
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER password_credentials_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.password_credentials
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER sessions_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.sessions
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

CREATE TRIGGER external_identities_reject_integration
    BEFORE INSERT OR UPDATE ON penfii.external_identities
    FOR EACH ROW EXECUTE FUNCTION penfii.reject_integration_membership();

-- ===== ticket archive =====

-- Ticket archive. `archived_at` already exists and is independent of `trashed_at` and `purged_at`;
-- this records who archived and adds the Sprint-completion archive schedule.

ALTER TABLE penfii.tickets
    ADD COLUMN archived_by uuid;

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_archived_by_fkey FOREIGN KEY (archived_by) REFERENCES penfii.users(id);

-- One row per Sprint at most: when the Tickets that ended DONE or CANCELED in that Sprint are archived.
-- Immediate archives run inside the completion command and write no row. A due SCHEDULED row is
-- picked up by the same polling loop pattern as reminders (SKIP LOCKED, one transaction per row), so
-- a server that was down at the due time archives afterwards instead of dropping the schedule.
CREATE TABLE penfii.sprint_archive_schedules (
    sprint_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    scheduled_by uuid NOT NULL,
    due_at timestamp with time zone NOT NULL,
    time_zone text NOT NULL,
    state text DEFAULT 'SCHEDULED'::text NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    archived_count integer,
    skipped_count integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    finished_at timestamp with time zone,
    CONSTRAINT sprint_archive_schedules_pkey PRIMARY KEY (sprint_id),
    CONSTRAINT sprint_archive_schedules_state_check CHECK ((state = ANY (ARRAY['SCHEDULED'::text, 'DONE'::text, 'CANCELED'::text]))),
    CONSTRAINT sprint_archive_schedules_sprint_fkey FOREIGN KEY (sprint_id) REFERENCES penfii.sprints(id),
    CONSTRAINT sprint_archive_schedules_project_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id),
    CONSTRAINT sprint_archive_schedules_user_fkey FOREIGN KEY (scheduled_by) REFERENCES penfii.users(id)
);

CREATE INDEX sprint_archive_schedules_due_idx ON penfii.sprint_archive_schedules USING btree (due_at, sprint_id) WHERE (state = 'SCHEDULED'::text);

-- ===== project channel membership cleanup =====

-- A person who no longer takes part in a Project has no membership in its channels, and getting the
-- Project back does not bring one back (FEA-061). From now on every change that ends a participation
-- clears those rows in the same transaction; this removes the rows that earlier changes left behind
-- (a removed grant, an unlinked Team, a Team the person left). The channel's access checks already
-- ignore such rows (FEA-060), so nothing that worked until now stops working: only the rows go, and
-- with them a Manager role that would otherwise return with the participation. Messages and the
-- per-person settings (read position, mutes, favorites, later items, notification settings) stay.
DELETE FROM penfii.channel_members cm
USING penfii.channels ch
WHERE ch.id = cm.channel_id
  AND ch.project_id IS NOT NULL
  AND NOT penfii.in_share_scope(ch.project_id, ch.team_id, cm.user_id);

-- ===== invitation reissue and delivery =====

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

-- ===== password reset tokens =====

-- FEA-063: password reset by e-mail. A token is a secret that is mailed once and stored only as its
-- hash. It works once (`closed_at` is set when it is used, replaced by a newer request or the password
-- changes), and only until `expires_at` (30 minutes after it was made).
CREATE TABLE penfii.password_reset_tokens (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES penfii.users(id),
    token_hash bytea NOT NULL UNIQUE,
    expires_at timestamp with time zone NOT NULL,
    closed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

-- At most one live token per account, whatever the order in which requests arrive.
CREATE UNIQUE INDEX password_reset_tokens_one_live_idx ON penfii.password_reset_tokens (user_id) WHERE closed_at IS NULL;
CREATE INDEX password_reset_tokens_expires_idx ON penfii.password_reset_tokens (expires_at);

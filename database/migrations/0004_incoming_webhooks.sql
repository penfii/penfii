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

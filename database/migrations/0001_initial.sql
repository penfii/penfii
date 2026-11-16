-- Penfii database schema, initial migration.
--
-- This is the complete schema as one file: it is the starting point of the migration history, and
-- every later change is a new numbered file next to it. The statements are pg_dump's, so the order
-- puts functions first; function bodies are therefore not checked when they are created.
--
-- sqlx runs each migration file once, in a transaction, and records its checksum: do not edit this
-- file after a release, add `0002_*.sql` instead.

SET LOCAL check_function_bodies = false;

CREATE SCHEMA IF NOT EXISTS penfii;

CREATE FUNCTION penfii.doc_pages_title_required() RETURNS trigger
    LANGUAGE plpgsql
    AS $$ BEGIN IF btrim(NEW.title) = '' AND (TG_OP = 'INSERT' OR NEW.title IS DISTINCT FROM OLD.title) THEN RAISE EXCEPTION 'doc title is required'; END IF; RETURN NEW; END; $$;

CREATE FUNCTION penfii.doc_versions_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$ BEGIN RAISE EXCEPTION 'doc_versions rows are immutable'; END; $$;

CREATE FUNCTION penfii.in_share_scope(p_project uuid, p_team uuid, p_user uuid) RETURNS boolean
    LANGUAGE sql STABLE
    AS $$ SELECT p_project IS NOT NULL AND (EXISTS (SELECT 1 FROM penfii.project_user_grants g WHERE g.project_id = p_project AND g.user_id = p_user) OR EXISTS (SELECT 1 FROM penfii.project_teams pt JOIN penfii.team_members tm ON tm.team_id = pt.team_id WHERE pt.project_id = p_project AND pt.access_role IS NOT NULL AND tm.user_id = p_user) OR EXISTS (SELECT 1 FROM penfii.project_group_grants pg JOIN penfii.groups gr ON gr.id = pg.group_id AND gr.deleted_at IS NULL LEFT JOIN penfii.group_members gm ON gm.group_id = gr.id LEFT JOIN penfii.team_members tm ON tm.team_id = gr.system_team_id WHERE pg.project_id = p_project AND (gm.user_id = p_user OR tm.user_id = p_user))) AND (p_team IS NULL OR EXISTS (SELECT 1 FROM penfii.team_members t WHERE t.team_id = p_team AND t.user_id = p_user)) $$;

CREATE TABLE penfii.activity_entries (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    origin_project_id uuid,
    origin_channel_id uuid,
    ticket_id uuid,
    ticket_revision_no bigint,
    actor_kind text NOT NULL,
    actor_user_id uuid,
    command_id uuid NOT NULL,
    activity_kind text NOT NULL,
    data_schema_version smallint DEFAULT 1 NOT NULL,
    data jsonb DEFAULT '{}'::jsonb NOT NULL,
    security_context jsonb DEFAULT '{}'::jsonb NOT NULL,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.audit_entries (
    id uuid NOT NULL,
    workspace_id uuid,
    actor_user_id uuid,
    action_code text NOT NULL,
    outcome_code text NOT NULL,
    target_kind text,
    target_id uuid,
    command_id uuid,
    data jsonb DEFAULT '{}'::jsonb NOT NULL,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.auth_attempts (
    id uuid NOT NULL,
    state_hash bytea NOT NULL,
    provider_id uuid NOT NULL,
    intent text NOT NULL,
    pkce_verifier text NOT NULL,
    nonce text NOT NULL,
    user_id uuid,
    session_id uuid,
    invitation_id uuid,
    return_to text DEFAULT '/'::text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT auth_attempts_intent_chk CHECK ((intent = ANY (ARRAY['login'::text, 'invite'::text, 'link'::text, 'reauth'::text])))
);

CREATE TABLE penfii.auth_providers (
    id uuid NOT NULL,
    provider_kind text NOT NULL,
    issuer text NOT NULL,
    client_id text NOT NULL,
    secret_ref text,
    enabled boolean DEFAULT false NOT NULL,
    config jsonb DEFAULT '{}'::jsonb NOT NULL,
    provider_key text,
    display_name text,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.auth_throttle (
    scope text NOT NULL,
    key_hash bytea NOT NULL,
    window_start timestamp with time zone DEFAULT now() NOT NULL,
    failures integer DEFAULT 0 NOT NULL,
    locked_until timestamp with time zone
);

CREATE TABLE penfii.channel_content_auditors (
    channel_id uuid NOT NULL,
    user_id uuid NOT NULL
);

CREATE TABLE penfii.channel_group_grants (
    channel_id uuid NOT NULL,
    group_id uuid NOT NULL,
    role text NOT NULL
);

CREATE TABLE penfii.channel_members (
    channel_id uuid NOT NULL,
    user_id uuid NOT NULL,
    role text NOT NULL,
    joined_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.channel_mutes (
    user_id uuid NOT NULL,
    channel_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.channels (
    id uuid NOT NULL,
    team_id uuid,
    project_id uuid,
    name text NOT NULL,
    description text,
    topic text,
    visibility text NOT NULL,
    allow_member_invites boolean DEFAULT false NOT NULL,
    archived_at timestamp with time zone,
    CONSTRAINT channels_visibility_chk CHECK ((visibility = ANY (ARRAY['PUBLIC'::text, 'PRIVATE'::text])))
);

CREATE TABLE penfii.commands (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    actor_key text NOT NULL,
    idempotency_key text NOT NULL,
    command_kind text NOT NULL,
    request_fingerprint bytea NOT NULL,
    result_schema_version smallint DEFAULT 1 NOT NULL,
    result_refs jsonb DEFAULT '[]'::jsonb NOT NULL,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.consumer_receipts (
    event_id uuid NOT NULL,
    consumer text NOT NULL,
    state text DEFAULT 'PENDING'::text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    available_at timestamp with time zone DEFAULT now() NOT NULL,
    lease_token uuid,
    lease_until timestamp with time zone,
    last_error_code text,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT consumer_receipts_state_check CHECK ((state = ANY (ARRAY['PENDING'::text, 'RUNNING'::text, 'DONE'::text, 'DEAD'::text])))
);

CREATE TABLE penfii.conversation_changes (
    conversation_id uuid NOT NULL,
    change_seq bigint NOT NULL,
    message_id uuid NOT NULL,
    message_version bigint,
    change_kind text NOT NULL,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.conversation_reads (
    conversation_id uuid NOT NULL,
    user_id uuid NOT NULL,
    read_through_seq bigint DEFAULT 0 NOT NULL,
    manual_unread_from_seq bigint,
    generation bigint DEFAULT 1 NOT NULL
);

CREATE TABLE penfii.conversations (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    kind text NOT NULL,
    last_message_seq bigint DEFAULT 0 NOT NULL,
    last_change_seq bigint DEFAULT 0 NOT NULL,
    replay_floor_seq bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT conversations_kind_chk CHECK ((kind = ANY (ARRAY['CHANNEL'::text, 'DM'::text])))
);

CREATE TABLE penfii.departments (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    parent_id uuid,
    name text NOT NULL,
    "position" integer DEFAULT 0 NOT NULL,
    CONSTRAINT departments_name_len CHECK (((char_length(btrim(name)) >= 1) AND (char_length(btrim(name)) <= 80))),
    CONSTRAINT departments_not_self CHECK ((parent_id IS DISTINCT FROM id))
);

CREATE TABLE penfii.dependency_waiver_history (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    dependency_id uuid NOT NULL,
    predecessor_id uuid NOT NULL,
    successor_id uuid NOT NULL,
    action text NOT NULL,
    reason text NOT NULL,
    actor_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT dependency_waiver_history_action_check CHECK ((action = ANY (ARRAY['WAIVED'::text, 'RESTORED'::text])))
);

CREATE TABLE penfii.dm_members (
    conversation_id uuid NOT NULL,
    user_id uuid NOT NULL
);

CREATE TABLE penfii.doc_collab_state (
    page_id uuid NOT NULL,
    epoch uuid NOT NULL,
    snapshot bytea NOT NULL,
    snapshot_seq bigint DEFAULT 0 NOT NULL,
    last_seq bigint DEFAULT 0 NOT NULL,
    materialized_seq bigint DEFAULT 0 NOT NULL,
    invalid_since timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.doc_collab_updates (
    page_id uuid NOT NULL,
    seq bigint NOT NULL,
    update bytea NOT NULL,
    actor_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.doc_comment_files (
    id uuid NOT NULL,
    comment_id uuid NOT NULL,
    file_id uuid NOT NULL,
    attached_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.doc_comments (
    id uuid NOT NULL,
    page_id uuid NOT NULL,
    author_id uuid NOT NULL,
    body jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    edited_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    deleted_at timestamp with time zone
);

CREATE TABLE penfii.doc_page_files (
    page_id uuid NOT NULL,
    file_id uuid NOT NULL,
    attached_by uuid,
    attached_at timestamp with time zone DEFAULT now() NOT NULL,
    unreferenced_since timestamp with time zone
);

CREATE TABLE penfii.doc_page_participants (
    page_id uuid NOT NULL,
    user_id uuid NOT NULL,
    access_role text NOT NULL,
    CONSTRAINT doc_page_participants_access_role_check CHECK ((access_role = ANY (ARRAY['MANAGER'::text, 'EDITOR'::text, 'VIEWER'::text])))
);

CREATE TABLE penfii.doc_page_targets (
    page_id uuid NOT NULL,
    target_kind text NOT NULL,
    target_id uuid NOT NULL,
    CONSTRAINT doc_page_targets_target_kind_check CHECK ((target_kind = ANY (ARRAY['TICKET'::text, 'DOC'::text, 'DATABASE'::text, 'DATABASE_RECORD'::text, 'MESSAGE'::text, 'FILE'::text, 'USER'::text])))
);

CREATE TABLE penfii.doc_pages (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid,
    parent_id uuid,
    title text NOT NULL,
    visibility text NOT NULL,
    "position" integer DEFAULT 0 NOT NULL,
    content jsonb NOT NULL,
    content_schema_version smallint NOT NULL,
    content_hash text NOT NULL,
    revision_no bigint DEFAULT 1 NOT NULL,
    materialized_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid,
    created_by uuid NOT NULL,
    deleted_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    scope_project_id uuid,
    scope_team_id uuid,
    CONSTRAINT doc_pages_scope_check CHECK ((((visibility = 'PROJECT'::text) AND (scope_project_id IS NOT NULL)) OR ((visibility <> 'PROJECT'::text) AND (scope_project_id IS NULL) AND (scope_team_id IS NULL)))),
    CONSTRAINT doc_pages_visibility_check CHECK ((visibility = ANY (ARRAY['WORKSPACE'::text, 'RESTRICTED'::text, 'PROJECT'::text])))
);

CREATE TABLE penfii.doc_templates (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    name text NOT NULL,
    content jsonb NOT NULL,
    content_schema_version smallint NOT NULL,
    created_by uuid NOT NULL,
    deleted_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.doc_version_files (
    page_id uuid NOT NULL,
    version_no integer NOT NULL,
    file_id uuid NOT NULL
);

CREATE TABLE penfii.doc_versions (
    page_id uuid NOT NULL,
    version_no integer NOT NULL,
    title text NOT NULL,
    content jsonb NOT NULL,
    content_schema_version smallint NOT NULL,
    content_hash text NOT NULL,
    source_revision_no bigint NOT NULL,
    created_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT doc_versions_version_no_check CHECK ((version_no >= 1))
);

CREATE TABLE penfii.domain_events (
    event_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    target_kind text NOT NULL,
    target_id uuid NOT NULL,
    target_version bigint,
    event_kind text NOT NULL,
    actor_kind text NOT NULL,
    actor_user_id uuid,
    initiated_by_user_id uuid,
    command_id uuid NOT NULL,
    schema_version smallint DEFAULT 1 NOT NULL,
    facts jsonb DEFAULT '{}'::jsonb NOT NULL,
    security_context jsonb DEFAULT '{}'::jsonb NOT NULL,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL,
    seq bigint NOT NULL
);

ALTER TABLE penfii.domain_events ALTER COLUMN seq ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME penfii.domain_events_seq_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE penfii.draft_conflicts (
    id uuid NOT NULL,
    draft_id uuid NOT NULL,
    generation uuid NOT NULL,
    base_version bigint NOT NULL,
    payload_schema_version smallint NOT NULL,
    payload jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.drafts (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    context_kind text NOT NULL,
    context_id uuid NOT NULL,
    generation uuid NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    consumed_version bigint DEFAULT 0 NOT NULL,
    state text DEFAULT 'ACTIVE'::text NOT NULL,
    payload_schema_version smallint NOT NULL,
    payload jsonb NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT drafts_state_chk CHECK ((state = ANY (ARRAY['ACTIVE'::text, 'LOCKED'::text, 'SENT'::text])))
);

CREATE TABLE penfii.event_recipients (
    event_id uuid NOT NULL,
    recipient_id uuid NOT NULL,
    notification_kind text NOT NULL,
    logical_key text NOT NULL,
    decision jsonb DEFAULT '{}'::jsonb NOT NULL
);

CREATE TABLE penfii.external_identities (
    issuer text NOT NULL,
    subject text NOT NULL,
    user_id uuid NOT NULL,
    provider_id uuid NOT NULL,
    linked_at timestamp with time zone DEFAULT now() NOT NULL,
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    email text,
    email_verified boolean,
    display_name text,
    avatar_url text,
    claims jsonb DEFAULT '{}'::jsonb NOT NULL,
    last_login_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.favorite_folders (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    name text NOT NULL,
    "position" bigint NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT favorite_folders_name_check CHECK (((char_length(name) >= 1) AND (char_length(name) <= 60)))
);

CREATE TABLE penfii.favorites (
    user_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    kind text NOT NULL,
    target_id uuid NOT NULL,
    "position" bigint NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    folder_id uuid,
    CONSTRAINT favorites_kind_check CHECK ((kind = ANY (ARRAY['PROJECT'::text, 'TEAM'::text, 'TICKET'::text, 'VIEW'::text, 'CHANNEL'::text])))
);

CREATE TABLE penfii.file_processing_results (
    task_id uuid NOT NULL,
    lease_token uuid NOT NULL,
    verdict text NOT NULL,
    artifacts jsonb DEFAULT '{}'::jsonb NOT NULL,
    error_code text,
    finished_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT file_processing_results_verdict_check CHECK ((verdict = ANY (ARRAY['CLEAN'::text, 'MALICIOUS'::text, 'ERROR'::text])))
);

CREATE TABLE penfii.file_processing_tasks (
    id uuid NOT NULL,
    file_id uuid NOT NULL,
    file_generation bigint NOT NULL,
    storage_key text NOT NULL,
    task_kind text NOT NULL,
    state text DEFAULT 'READY'::text NOT NULL,
    available_at timestamp with time zone DEFAULT now() NOT NULL,
    lease_token uuid,
    lease_until timestamp with time zone,
    attempts integer DEFAULT 0 NOT NULL,
    CONSTRAINT file_processing_tasks_check CHECK ((((state = 'RUNNING'::text) AND (lease_token IS NOT NULL) AND (lease_until IS NOT NULL)) OR ((state <> 'RUNNING'::text) AND (lease_token IS NULL) AND (lease_until IS NULL)))),
    CONSTRAINT file_processing_tasks_state_check CHECK ((state = ANY (ARRAY['READY'::text, 'RUNNING'::text, 'DONE'::text, 'FAILED'::text, 'CANCELED'::text])))
);

CREATE TABLE penfii.files (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    storage_key text NOT NULL,
    filename text NOT NULL,
    byte_size bigint NOT NULL,
    detected_mime text,
    state text NOT NULL,
    generation bigint DEFAULT 1 NOT NULL,
    created_by uuid NOT NULL,
    deleted_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT files_byte_size_check CHECK ((byte_size >= 0)),
    CONSTRAINT files_state_check CHECK ((state = ANY (ARRAY['UPLOADING'::text, 'QUARANTINED'::text, 'AVAILABLE'::text, 'REJECTED'::text])))
);

CREATE TABLE penfii.foundation_state (
    key text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.group_members (
    group_id uuid NOT NULL,
    user_id uuid NOT NULL
);

CREATE TABLE penfii.groups (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    name text NOT NULL,
    system_team_id uuid,
    deleted_at timestamp with time zone
);

CREATE TABLE penfii.inbox_items (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    event_id uuid NOT NULL,
    recipient_id uuid NOT NULL,
    notification_kind text NOT NULL,
    logical_key text NOT NULL,
    state text DEFAULT 'UNREAD'::text NOT NULL,
    snoozed_until timestamp with time zone,
    version bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT inbox_items_state_check CHECK ((state = ANY (ARRAY['UNREAD'::text, 'READ'::text, 'ARCHIVED'::text])))
);

CREATE TABLE penfii.instance_settings (
    singleton boolean DEFAULT true NOT NULL,
    default_locale text DEFAULT 'ko-KR'::text NOT NULL,
    CONSTRAINT instance_settings_singleton_chk CHECK (singleton)
);

CREATE TABLE penfii.labels (
    id uuid NOT NULL,
    project_id uuid NOT NULL,
    name text NOT NULL,
    color text,
    retired_at timestamp with time zone
);

CREATE TABLE penfii.later_items (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    kind text NOT NULL,
    target_id uuid NOT NULL,
    note text,
    state text DEFAULT 'ACTIVE'::text NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT later_items_kind_check CHECK ((kind = ANY (ARRAY['TICKET'::text, 'MESSAGE'::text, 'FILE'::text]))),
    CONSTRAINT later_items_state_check CHECK ((state = ANY (ARRAY['ACTIVE'::text, 'DONE'::text, 'ARCHIVED'::text])))
);

CREATE TABLE penfii.mcp_call_log (
    id uuid NOT NULL,
    at timestamp with time zone DEFAULT now() NOT NULL,
    user_id uuid NOT NULL,
    grant_id uuid,
    client_pk uuid,
    tool text NOT NULL,
    operation text NOT NULL,
    risk_class text NOT NULL,
    workspace_id uuid,
    resource_type text,
    resource_id text,
    request_id text NOT NULL,
    outcome text NOT NULL,
    duration_ms integer DEFAULT 0 NOT NULL,
    resource_revision bigint
);

CREATE TABLE penfii.message_attachments (
    id uuid NOT NULL,
    message_id uuid NOT NULL,
    file_id uuid NOT NULL
);

CREATE TABLE penfii.message_mentions (
    message_id uuid NOT NULL,
    occurrence_id uuid NOT NULL,
    mention_kind text NOT NULL,
    user_id uuid,
    group_id uuid,
    removed_at timestamp with time zone,
    CONSTRAINT message_mentions_kind_chk CHECK ((mention_kind = ANY (ARRAY['USER'::text, 'GROUP'::text, 'TEAM'::text, 'CHANNEL'::text, 'HERE'::text]))),
    CONSTRAINT message_mentions_target_chk CHECK ((((mention_kind = 'USER'::text) AND (user_id IS NOT NULL) AND (group_id IS NULL)) OR ((mention_kind = ANY (ARRAY['GROUP'::text, 'TEAM'::text])) AND (group_id IS NOT NULL) AND (user_id IS NULL)) OR ((mention_kind = ANY (ARRAY['CHANNEL'::text, 'HERE'::text])) AND (user_id IS NULL) AND (group_id IS NULL))))
);

CREATE TABLE penfii.message_reactions (
    message_id uuid NOT NULL,
    user_id uuid NOT NULL,
    emoji text NOT NULL
);

CREATE TABLE penfii.message_version_files (
    message_id uuid NOT NULL,
    version bigint NOT NULL,
    file_id uuid NOT NULL
);

CREATE TABLE penfii.message_versions (
    message_id uuid NOT NULL,
    version bigint NOT NULL,
    body_schema_version smallint NOT NULL,
    body jsonb NOT NULL,
    changed_by uuid NOT NULL,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.messages (
    id uuid NOT NULL,
    conversation_id uuid NOT NULL,
    stream_seq bigint NOT NULL,
    author_id uuid NOT NULL,
    thread_root_id uuid,
    reply_to_id uuid,
    version bigint DEFAULT 1 NOT NULL,
    body_schema_version smallint NOT NULL,
    body jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    edited_at timestamp with time zone,
    deleted_at timestamp with time zone,
    CONSTRAINT messages_reply_to_chk CHECK (((reply_to_id IS NULL) OR (reply_to_id <> id))),
    CONSTRAINT messages_stream_seq_chk CHECK ((stream_seq > 0)),
    CONSTRAINT messages_thread_root_chk CHECK (((thread_root_id IS NULL) OR (thread_root_id <> id))),
    CONSTRAINT messages_version_chk CHECK ((version > 0))
);

CREATE TABLE penfii.milestones (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    name text NOT NULL,
    description text,
    target_on date,
    state text DEFAULT 'OPEN'::text NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    reached_at timestamp with time zone,
    CONSTRAINT milestones_state_check CHECK ((state = ANY (ARRAY['OPEN'::text, 'REACHED'::text, 'CANCELED'::text])))
);

CREATE TABLE penfii.notification_preferences (
    user_id uuid NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    settings jsonb DEFAULT '{}'::jsonb NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.oauth_authorization_requests (
    id uuid NOT NULL,
    client_pk uuid NOT NULL,
    user_id uuid,
    redirect_uri text NOT NULL,
    state text,
    code_challenge text NOT NULL,
    scope_requested text[] DEFAULT '{}'::text[] NOT NULL,
    resource text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.oauth_clients (
    id uuid NOT NULL,
    client_id text NOT NULL,
    kind text NOT NULL,
    client_name text NOT NULL,
    redirect_uris jsonb NOT NULL,
    token_endpoint_auth_method text DEFAULT 'none'::text NOT NULL,
    jwks_uri text,
    metadata_hash bytea,
    fetched_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone,
    disabled_at timestamp with time zone,
    CONSTRAINT oauth_clients_kind_check CHECK ((kind = ANY (ARRAY['cimd'::text, 'dcr'::text, 'preregistered'::text])))
);

CREATE TABLE penfii.oauth_codes (
    code_hash bytea NOT NULL,
    grant_id uuid NOT NULL,
    client_pk uuid NOT NULL,
    redirect_uri text NOT NULL,
    code_challenge text NOT NULL,
    resource text NOT NULL,
    scope text[] DEFAULT '{}'::text[] NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone,
    family_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.oauth_grants (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    client_pk uuid NOT NULL,
    scope text[] DEFAULT '{}'::text[] NOT NULL,
    workspace_ids uuid[],
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_used_at timestamp with time zone,
    revoked_at timestamp with time zone,
    revoked_reason text
);

CREATE TABLE penfii.oauth_tokens (
    id uuid NOT NULL,
    grant_id uuid NOT NULL,
    family_id uuid NOT NULL,
    kind text NOT NULL,
    token_hash bytea NOT NULL,
    scope text[] DEFAULT '{}'::text[] NOT NULL,
    resource text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    absolute_expires_at timestamp with time zone,
    used_at timestamp with time zone,
    revoked_at timestamp with time zone,
    parent_id uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_used_at timestamp with time zone,
    CONSTRAINT oauth_tokens_kind_check CHECK ((kind = ANY (ARRAY['access'::text, 'refresh'::text])))
);

CREATE TABLE penfii.outbox (
    event_id uuid NOT NULL,
    available_at timestamp with time zone DEFAULT now() NOT NULL,
    dispatched_at timestamp with time zone,
    attempts integer DEFAULT 0 NOT NULL,
    last_error_code text
);

CREATE TABLE penfii.password_credentials (
    user_id uuid NOT NULL,
    password_hash text NOT NULL,
    changed_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.project_calendars (
    project_id uuid NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    time_zone text NOT NULL,
    definition jsonb NOT NULL,
    updated_by uuid,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.project_group_grants (
    project_id uuid NOT NULL,
    group_id uuid NOT NULL,
    role text NOT NULL
);

CREATE TABLE penfii.project_pins (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    kind text NOT NULL,
    ticket_id uuid,
    url text,
    title text,
    "position" bigint NOT NULL,
    pinned_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT project_pins_check CHECK ((((kind = 'TICKET'::text) AND (ticket_id IS NOT NULL) AND (url IS NULL)) OR ((kind = 'URL'::text) AND (url IS NOT NULL) AND (ticket_id IS NULL)))),
    CONSTRAINT project_pins_kind_check CHECK ((kind = ANY (ARRAY['TICKET'::text, 'URL'::text])))
);

CREATE TABLE penfii.project_teams (
    project_id uuid NOT NULL,
    team_id uuid NOT NULL,
    access_role text,
    CONSTRAINT project_teams_access_role_check CHECK (((access_role IS NULL) OR (access_role = ANY (ARRAY['ADMIN'::text, 'EDITOR'::text, 'VIEWER'::text]))))
);

CREATE TABLE penfii.project_ticket_changes (
    project_id uuid NOT NULL,
    change_seq bigint NOT NULL,
    ticket_id uuid NOT NULL,
    ticket_revision_no bigint NOT NULL,
    change_kind text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.project_ticket_order (
    ticket_id uuid NOT NULL,
    project_id uuid NOT NULL,
    sort_key numeric NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    moved_to_top_at timestamp with time zone
);

CREATE TABLE penfii.project_user_grants (
    project_id uuid NOT NULL,
    user_id uuid NOT NULL,
    role text NOT NULL,
    can_waive_dependencies boolean DEFAULT false NOT NULL,
    can_manage_pins boolean DEFAULT false NOT NULL
);

CREATE TABLE penfii.project_views (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid,
    owner_id uuid NOT NULL,
    scope text NOT NULL,
    name text NOT NULL,
    layout text NOT NULL,
    definition_schema_version smallint DEFAULT 1 NOT NULL,
    definition jsonb NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    deleted_at timestamp with time zone,
    target_kind text DEFAULT 'PROJECT'::text NOT NULL,
    team_id uuid,
    CONSTRAINT project_views_layout_chk CHECK ((layout = ANY (ARRAY['BOARD'::text, 'LIST'::text, 'TABLE'::text]))),
    CONSTRAINT project_views_scope_check CHECK ((scope = ANY (ARRAY['PERSONAL'::text, 'PROJECT'::text]))),
    CONSTRAINT project_views_target_chk CHECK ((((target_kind = 'PROJECT'::text) AND (project_id IS NOT NULL) AND (team_id IS NULL)) OR ((target_kind = 'TEAM'::text) AND (project_id IS NULL) AND (team_id IS NOT NULL) AND (scope = 'PERSONAL'::text)) OR ((target_kind = 'WORKSPACE'::text) AND (project_id IS NULL) AND (team_id IS NULL) AND (scope = 'PERSONAL'::text))))
);

CREATE TABLE penfii.projects (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    name text NOT NULL,
    estimate_unit text DEFAULT 'NONE'::text NOT NULL,
    archived_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    ticket_prefix text NOT NULL,
    ticket_seq bigint DEFAULT 0 NOT NULL,
    CONSTRAINT projects_estimate_unit_chk CHECK ((estimate_unit = ANY (ARRAY['NONE'::text, 'POINTS'::text, 'HOURS'::text]))),
    CONSTRAINT projects_ticket_prefix_chk CHECK (((ticket_prefix IS NULL) OR (ticket_prefix ~ '^[A-Z][A-Z0-9]{1,9}$'::text)))
);

CREATE TABLE penfii.reminders (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    target_kind text NOT NULL,
    target_id uuid NOT NULL,
    later_id uuid NOT NULL,
    due_at timestamp with time zone NOT NULL,
    time_zone text NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    state text DEFAULT 'SCHEDULED'::text NOT NULL,
    cancel_reason text,
    fired_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT reminders_state_check CHECK ((state = ANY (ARRAY['SCHEDULED'::text, 'FIRED'::text, 'CANCELED'::text])))
);

CREATE TABLE penfii.search_index_state (
    kind text NOT NULL,
    mapping_version integer NOT NULL,
    live_index text,
    building_index text,
    phase text DEFAULT 'EMPTY'::text NOT NULL,
    checkpoint jsonb DEFAULT '{}'::jsonb NOT NULL,
    last_error_code text,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT search_index_state_phase_check CHECK ((phase = ANY (ARRAY['EMPTY'::text, 'BUILDING'::text, 'READY'::text, 'STALE_MAPPING'::text])))
);

CREATE TABLE penfii.sessions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    token_hash bytea NOT NULL,
    device_label text,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    authenticated_at timestamp with time zone,
    last_reauth_at timestamp with time zone,
    last_reauth_method text,
    auth_method text,
    last_seen_at timestamp with time zone
);

CREATE TABLE penfii.sprint_assignments (
    id uuid NOT NULL,
    sprint_id uuid NOT NULL,
    ticket_id uuid NOT NULL,
    assigned_at timestamp with time zone DEFAULT now() NOT NULL,
    assigned_by uuid NOT NULL,
    ended_at timestamp with time zone,
    end_reason text,
    outcome text,
    carried_to_sprint_id uuid,
    carried_from_sprint_id uuid,
    needs_next_assignment boolean DEFAULT false NOT NULL,
    ended_status_category text,
    ended_estimate_value numeric(10,2),
    ended_estimate_unit text,
    CONSTRAINT sprint_assignments_check CHECK (((ended_at IS NULL) OR (ended_at >= assigned_at))),
    CONSTRAINT sprint_assignments_check1 CHECK (((ended_at IS NULL) = (end_reason IS NULL))),
    CONSTRAINT sprint_assignments_outcome_check CHECK ((outcome = ANY (ARRAY['DONE'::text, 'CANCELED'::text, 'NOT_DONE'::text])))
);

CREATE TABLE penfii.sprint_baseline_tickets (
    sprint_id uuid NOT NULL,
    ticket_id uuid NOT NULL,
    ticket_revision_no bigint,
    status_id uuid,
    status_category text,
    estimate_value numeric(10,2),
    estimate_unit text,
    hierarchy_kind text
);

CREATE TABLE penfii.sprint_baselines (
    sprint_id uuid NOT NULL,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL,
    recorded_by uuid NOT NULL,
    task_count integer NOT NULL,
    snapshot_version smallint DEFAULT 1 NOT NULL,
    estimate_unit text,
    time_zone text,
    policy_version smallint
);

CREATE TABLE penfii.sprints (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    name text NOT NULL,
    goal text,
    state text DEFAULT 'PLANNED'::text NOT NULL,
    starts_on date NOT NULL,
    ends_on date NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    started_at timestamp with time zone,
    ended_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    time_zone text NOT NULL,
    estimate_unit text NOT NULL,
    CONSTRAINT sprints_check CHECK ((ends_on >= starts_on)),
    CONSTRAINT sprints_estimate_unit_check CHECK ((estimate_unit = ANY (ARRAY['NONE'::text, 'POINTS'::text, 'HOURS'::text]))),
    CONSTRAINT sprints_state_check CHECK ((state = ANY (ARRAY['PLANNED'::text, 'ACTIVE'::text, 'COMPLETED'::text, 'CANCELED'::text])))
);

CREATE TABLE penfii.subscriptions (
    user_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    target_kind text NOT NULL,
    target_id uuid NOT NULL,
    mode text NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    options jsonb DEFAULT '{}'::jsonb NOT NULL,
    initialized_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT subscriptions_mode_check CHECK ((mode = ANY (ARRAY['AUTO'::text, 'EXPLICIT'::text, 'OPT_OUT'::text]))),
    CONSTRAINT subscriptions_target_kind_check CHECK ((target_kind = ANY (ARRAY['TICKET'::text, 'PROJECT'::text, 'THREAD'::text])))
);

CREATE TABLE penfii.team_members (
    team_id uuid NOT NULL,
    user_id uuid NOT NULL,
    role text NOT NULL,
    CONSTRAINT team_members_role_check CHECK ((role = ANY (ARRAY['MANAGER'::text, 'MEMBER'::text])))
);

CREATE TABLE penfii.teams (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    name text NOT NULL,
    archived_at timestamp with time zone,
    department_id uuid,
    description text,
    CONSTRAINT teams_description_len CHECK (((description IS NULL) OR (char_length(description) <= 2000)))
);

CREATE TABLE penfii.thread_reads (
    thread_root_id uuid NOT NULL,
    user_id uuid NOT NULL,
    read_through_seq bigint DEFAULT 0 NOT NULL,
    manual_unread_from_seq bigint,
    generation bigint DEFAULT 1 NOT NULL
);

CREATE TABLE penfii.ticket_attachments (
    id uuid NOT NULL,
    ticket_id uuid NOT NULL,
    comment_id uuid,
    file_id uuid NOT NULL,
    attached_by uuid NOT NULL,
    attached_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.ticket_comments (
    id uuid NOT NULL,
    ticket_id uuid NOT NULL,
    author_id uuid NOT NULL,
    reply_to_id uuid,
    version bigint DEFAULT 1 NOT NULL,
    body_schema_version smallint DEFAULT 1 NOT NULL,
    body jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    edited_at timestamp with time zone,
    deleted_at timestamp with time zone
);

CREATE TABLE penfii.ticket_dependencies (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    predecessor_id uuid NOT NULL,
    successor_id uuid NOT NULL,
    created_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    kind text DEFAULT 'BLOCKS'::text NOT NULL,
    waived_at timestamp with time zone,
    waived_by uuid,
    waive_reason text,
    CONSTRAINT ticket_dependencies_check CHECK ((predecessor_id <> successor_id)),
    CONSTRAINT ticket_dependencies_kind_check CHECK ((kind = ANY (ARRAY['BLOCKS'::text, 'RELATED'::text])))
);

CREATE TABLE penfii.ticket_labels (
    ticket_id uuid NOT NULL,
    label_id uuid NOT NULL
);

CREATE TABLE penfii.ticket_milestones (
    ticket_id uuid NOT NULL,
    milestone_id uuid NOT NULL,
    linked_by uuid NOT NULL,
    linked_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.ticket_revisions (
    ticket_id uuid NOT NULL,
    revision_no bigint NOT NULL,
    actor_kind text NOT NULL,
    actor_user_id uuid,
    initiated_by_user_id uuid,
    command_id uuid NOT NULL,
    snapshot_schema_version smallint NOT NULL,
    snapshot jsonb NOT NULL,
    security_context jsonb DEFAULT '{}'::jsonb NOT NULL,
    restore_source_revision_no bigint,
    restore_metadata jsonb,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT ticket_revisions_actor_kind_check CHECK ((actor_kind = ANY (ARRAY['USER'::text, 'SYSTEM'::text])))
);

CREATE TABLE penfii.ticket_schedules (
    ticket_id uuid NOT NULL,
    planned_start_at timestamp with time zone,
    duration_minutes bigint,
    is_milestone boolean DEFAULT false NOT NULL,
    CONSTRAINT ticket_schedules_check CHECK (((duration_minutes IS NULL) OR (duration_minutes > 0) OR ((duration_minutes = 0) AND is_milestone)))
);

CREATE TABLE penfii.ticket_types (
    id uuid NOT NULL,
    project_id uuid NOT NULL,
    name text NOT NULL,
    description text,
    color text,
    retired_at timestamp with time zone
);

CREATE TABLE penfii.tickets (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    channel_id uuid,
    public_key text NOT NULL,
    title text NOT NULL,
    hierarchy_kind text NOT NULL,
    parent_id uuid,
    ticket_type_id uuid,
    status_id uuid NOT NULL,
    assignee_id uuid,
    created_by uuid NOT NULL,
    priority text,
    due_on date,
    description_schema_version smallint DEFAULT 1 NOT NULL,
    description jsonb DEFAULT '{}'::jsonb NOT NULL,
    revision_no bigint DEFAULT 1 NOT NULL,
    archived_at timestamp with time zone,
    trashed_at timestamp with time zone,
    purged_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    project_sequence bigint NOT NULL,
    estimate_value numeric(10,2),
    estimate_unit text,
    trashed_by uuid,
    purged_by uuid,
    CONSTRAINT tickets_check CHECK (((parent_id IS NULL) OR (parent_id <> id))),
    CONSTRAINT tickets_check1 CHECK (((hierarchy_kind <> 'EPIC'::text) OR ((parent_id IS NULL) AND (channel_id IS NULL)))),
    CONSTRAINT tickets_check2 CHECK (((hierarchy_kind <> 'SUBTASK'::text) OR (parent_id IS NOT NULL))),
    CONSTRAINT tickets_check3 CHECK ((((estimate_unit IS NULL) = (estimate_value IS NULL)) AND ((estimate_unit IS NULL) OR (estimate_unit = ANY (ARRAY['POINTS'::text, 'HOURS'::text]))))),
    CONSTRAINT tickets_estimate_value_check CHECK (((estimate_value IS NULL) OR (estimate_value >= (0)::numeric))),
    CONSTRAINT tickets_hierarchy_kind_check CHECK ((hierarchy_kind = ANY (ARRAY['EPIC'::text, 'TASK'::text, 'SUBTASK'::text]))),
    CONSTRAINT tickets_revision_no_check CHECK ((revision_no > 0))
);

CREATE TABLE penfii.users (
    id uuid NOT NULL,
    email text NOT NULL,
    email_key text NOT NULL,
    email_verified_at timestamp with time zone,
    display_name text NOT NULL,
    locale text,
    time_zone text,
    disabled_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    username text,
    username_key text,
    job_title text,
    status_emoji text,
    status_text text,
    status_clear_at timestamp with time zone,
    avatar_object_key text,
    avatar_mime text,
    avatar_revision bigint DEFAULT 0 NOT NULL,
    CONSTRAINT users_avatar_key_len CHECK (((avatar_object_key IS NULL) OR (char_length(avatar_object_key) <= 200))),
    CONSTRAINT users_avatar_pair CHECK (((avatar_object_key IS NULL) = (avatar_mime IS NULL))),
    CONSTRAINT users_avatar_revision_nonneg CHECK ((avatar_revision >= 0)),
    CONSTRAINT users_job_title_len CHECK (((job_title IS NULL) OR (char_length(job_title) <= 80))),
    CONSTRAINT users_status_emoji_len CHECK (((status_emoji IS NULL) OR (char_length(status_emoji) <= 16))),
    CONSTRAINT users_status_text_len CHECK (((status_text IS NULL) OR (char_length(status_text) <= 80)))
);

CREATE TABLE penfii.workflow_status_history (
    id bigint NOT NULL,
    status_id uuid NOT NULL,
    category text NOT NULL,
    effective_from timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT workflow_status_history_category_check CHECK ((category = ANY (ARRAY['NOT_STARTED'::text, 'IN_PROGRESS'::text, 'DONE'::text, 'CANCELED'::text])))
);

ALTER TABLE penfii.workflow_status_history ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME penfii.workflow_status_history_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE penfii.workflow_statuses (
    id uuid NOT NULL,
    project_id uuid NOT NULL,
    name text NOT NULL,
    category text NOT NULL,
    "position" integer NOT NULL,
    is_initial boolean DEFAULT false NOT NULL,
    retired_at timestamp with time zone,
    wip_limit integer,
    CONSTRAINT workflow_statuses_category_check CHECK ((category = ANY (ARRAY['NOT_STARTED'::text, 'IN_PROGRESS'::text, 'DONE'::text, 'CANCELED'::text]))),
    CONSTRAINT workflow_statuses_check CHECK (((NOT is_initial) OR ((category = 'NOT_STARTED'::text) AND (retired_at IS NULL)))),
    CONSTRAINT workflow_statuses_wip_limit_check CHECK (((wip_limit IS NULL) OR (wip_limit > 0)))
);

CREATE TABLE penfii.workflow_transitions (
    from_status_id uuid NOT NULL,
    to_status_id uuid NOT NULL,
    CONSTRAINT workflow_transitions_check CHECK ((from_status_id <> to_status_id))
);

CREATE TABLE penfii.workflows (
    project_id uuid NOT NULL,
    version bigint DEFAULT 1 NOT NULL
);

CREATE TABLE penfii.workspace_auth_state (
    workspace_id uuid NOT NULL,
    version bigint DEFAULT 1 NOT NULL
);

CREATE TABLE penfii.workspace_graph_state (
    workspace_id uuid NOT NULL,
    version bigint DEFAULT 1 NOT NULL
);

CREATE TABLE penfii.workspace_invitations (
    id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    email_key text NOT NULL,
    role text NOT NULL,
    token_hash bytea NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_by uuid NOT NULL,
    accepted_user_id uuid,
    accepted_at timestamp with time zone,
    revoked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    team_id uuid,
    team_name text
);

CREATE TABLE penfii.workspace_mcp_policy (
    workspace_id uuid NOT NULL,
    mcp_enabled boolean NOT NULL,
    allow_write boolean NOT NULL,
    allow_destructive boolean NOT NULL,
    allow_administrative boolean NOT NULL,
    updated_by uuid,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE penfii.workspace_members (
    workspace_id uuid NOT NULL,
    user_id uuid NOT NULL,
    role text NOT NULL,
    state text DEFAULT 'ACTIVE'::text NOT NULL,
    deactivated_at timestamp with time zone,
    joined_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT workspace_members_state_chk CHECK ((state = ANY (ARRAY['ACTIVE'::text, 'INACTIVE'::text])))
);

CREATE TABLE penfii.workspace_security_auditors (
    workspace_id uuid NOT NULL,
    user_id uuid NOT NULL
);

CREATE TABLE penfii.workspaces (
    id uuid NOT NULL,
    name text NOT NULL,
    time_zone text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

ALTER TABLE ONLY penfii.activity_entries
    ADD CONSTRAINT activity_entries_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.audit_entries
    ADD CONSTRAINT audit_entries_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.auth_attempts
    ADD CONSTRAINT auth_attempts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.auth_attempts
    ADD CONSTRAINT auth_attempts_state_hash_key UNIQUE (state_hash);

ALTER TABLE ONLY penfii.auth_providers
    ADD CONSTRAINT auth_providers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.auth_throttle
    ADD CONSTRAINT auth_throttle_pkey PRIMARY KEY (scope, key_hash);

ALTER TABLE ONLY penfii.channel_content_auditors
    ADD CONSTRAINT channel_content_auditors_pkey PRIMARY KEY (channel_id, user_id);

ALTER TABLE ONLY penfii.channel_group_grants
    ADD CONSTRAINT channel_group_grants_pkey PRIMARY KEY (channel_id, group_id);

ALTER TABLE ONLY penfii.channel_members
    ADD CONSTRAINT channel_members_pkey PRIMARY KEY (channel_id, user_id);

ALTER TABLE ONLY penfii.channel_mutes
    ADD CONSTRAINT channel_mutes_pkey PRIMARY KEY (user_id, channel_id);

ALTER TABLE ONLY penfii.channels
    ADD CONSTRAINT channels_pkey PRIMARY KEY (id);

ALTER TABLE penfii.channels
    ADD CONSTRAINT channels_team_needs_project_chk CHECK (((team_id IS NULL) OR (project_id IS NOT NULL))) NOT VALID;

ALTER TABLE ONLY penfii.commands
    ADD CONSTRAINT commands_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.commands
    ADD CONSTRAINT commands_workspace_id_actor_key_idempotency_key_key UNIQUE (workspace_id, actor_key, idempotency_key);

ALTER TABLE ONLY penfii.consumer_receipts
    ADD CONSTRAINT consumer_receipts_pkey PRIMARY KEY (event_id, consumer);

ALTER TABLE ONLY penfii.conversation_changes
    ADD CONSTRAINT conversation_changes_pkey PRIMARY KEY (conversation_id, change_seq);

ALTER TABLE ONLY penfii.conversation_reads
    ADD CONSTRAINT conversation_reads_pkey PRIMARY KEY (conversation_id, user_id);

ALTER TABLE ONLY penfii.conversations
    ADD CONSTRAINT conversations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.departments
    ADD CONSTRAINT departments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.dependency_waiver_history
    ADD CONSTRAINT dependency_waiver_history_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.dm_members
    ADD CONSTRAINT dm_members_pkey PRIMARY KEY (conversation_id, user_id);

ALTER TABLE ONLY penfii.doc_collab_state
    ADD CONSTRAINT doc_collab_state_pkey PRIMARY KEY (page_id);

ALTER TABLE ONLY penfii.doc_collab_updates
    ADD CONSTRAINT doc_collab_updates_pkey PRIMARY KEY (page_id, seq);

ALTER TABLE ONLY penfii.doc_comment_files
    ADD CONSTRAINT doc_comment_files_file_id_key UNIQUE (file_id);

ALTER TABLE ONLY penfii.doc_comment_files
    ADD CONSTRAINT doc_comment_files_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.doc_comments
    ADD CONSTRAINT doc_comments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.doc_page_files
    ADD CONSTRAINT doc_page_files_pkey PRIMARY KEY (page_id, file_id);

ALTER TABLE ONLY penfii.doc_page_participants
    ADD CONSTRAINT doc_page_participants_pkey PRIMARY KEY (page_id, user_id);

ALTER TABLE ONLY penfii.doc_page_targets
    ADD CONSTRAINT doc_page_targets_pkey PRIMARY KEY (page_id, target_kind, target_id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.doc_templates
    ADD CONSTRAINT doc_templates_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.doc_version_files
    ADD CONSTRAINT doc_version_files_pkey PRIMARY KEY (page_id, version_no, file_id);

ALTER TABLE ONLY penfii.doc_versions
    ADD CONSTRAINT doc_versions_pkey PRIMARY KEY (page_id, version_no);

ALTER TABLE ONLY penfii.domain_events
    ADD CONSTRAINT domain_events_pkey PRIMARY KEY (event_id);

ALTER TABLE ONLY penfii.draft_conflicts
    ADD CONSTRAINT draft_conflicts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.drafts
    ADD CONSTRAINT drafts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.drafts
    ADD CONSTRAINT drafts_user_id_workspace_id_context_kind_context_id_key UNIQUE (user_id, workspace_id, context_kind, context_id);

ALTER TABLE ONLY penfii.event_recipients
    ADD CONSTRAINT event_recipients_pkey PRIMARY KEY (event_id, recipient_id, notification_kind);

ALTER TABLE ONLY penfii.external_identities
    ADD CONSTRAINT external_identities_pkey PRIMARY KEY (issuer, subject);

ALTER TABLE ONLY penfii.favorite_folders
    ADD CONSTRAINT favorite_folders_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.favorites
    ADD CONSTRAINT favorites_pkey PRIMARY KEY (user_id, kind, target_id);

ALTER TABLE ONLY penfii.file_processing_results
    ADD CONSTRAINT file_processing_results_pkey PRIMARY KEY (task_id, lease_token);

ALTER TABLE ONLY penfii.file_processing_tasks
    ADD CONSTRAINT file_processing_tasks_file_id_file_generation_task_kind_key UNIQUE (file_id, file_generation, task_kind);

ALTER TABLE ONLY penfii.file_processing_tasks
    ADD CONSTRAINT file_processing_tasks_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.files
    ADD CONSTRAINT files_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.files
    ADD CONSTRAINT files_storage_key_key UNIQUE (storage_key);

ALTER TABLE ONLY penfii.foundation_state
    ADD CONSTRAINT foundation_state_pkey PRIMARY KEY (key);

ALTER TABLE ONLY penfii.group_members
    ADD CONSTRAINT group_members_pkey PRIMARY KEY (group_id, user_id);

ALTER TABLE ONLY penfii.groups
    ADD CONSTRAINT groups_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.groups
    ADD CONSTRAINT groups_system_team_id_key UNIQUE (system_team_id);

ALTER TABLE ONLY penfii.inbox_items
    ADD CONSTRAINT inbox_items_event_id_recipient_id_notification_kind_key UNIQUE (event_id, recipient_id, notification_kind);

ALTER TABLE ONLY penfii.inbox_items
    ADD CONSTRAINT inbox_items_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.inbox_items
    ADD CONSTRAINT inbox_items_recipient_id_logical_key_key UNIQUE (recipient_id, logical_key);

ALTER TABLE ONLY penfii.instance_settings
    ADD CONSTRAINT instance_settings_pkey PRIMARY KEY (singleton);

ALTER TABLE ONLY penfii.labels
    ADD CONSTRAINT labels_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.later_items
    ADD CONSTRAINT later_items_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.mcp_call_log
    ADD CONSTRAINT mcp_call_log_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.message_attachments
    ADD CONSTRAINT message_attachments_message_id_file_id_key UNIQUE (message_id, file_id);

ALTER TABLE ONLY penfii.message_attachments
    ADD CONSTRAINT message_attachments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.message_mentions
    ADD CONSTRAINT message_mentions_pkey PRIMARY KEY (message_id, occurrence_id);

ALTER TABLE ONLY penfii.message_reactions
    ADD CONSTRAINT message_reactions_pkey PRIMARY KEY (message_id, user_id, emoji);

ALTER TABLE ONLY penfii.message_version_files
    ADD CONSTRAINT message_version_files_pkey PRIMARY KEY (message_id, version, file_id);

ALTER TABLE ONLY penfii.message_versions
    ADD CONSTRAINT message_versions_pkey PRIMARY KEY (message_id, version);

ALTER TABLE ONLY penfii.messages
    ADD CONSTRAINT messages_conversation_id_stream_seq_key UNIQUE (conversation_id, stream_seq);

ALTER TABLE ONLY penfii.messages
    ADD CONSTRAINT messages_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.milestones
    ADD CONSTRAINT milestones_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.notification_preferences
    ADD CONSTRAINT notification_preferences_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY penfii.oauth_authorization_requests
    ADD CONSTRAINT oauth_authorization_requests_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.oauth_clients
    ADD CONSTRAINT oauth_clients_client_id_key UNIQUE (client_id);

ALTER TABLE ONLY penfii.oauth_clients
    ADD CONSTRAINT oauth_clients_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.oauth_codes
    ADD CONSTRAINT oauth_codes_pkey PRIMARY KEY (code_hash);

ALTER TABLE ONLY penfii.oauth_grants
    ADD CONSTRAINT oauth_grants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.oauth_tokens
    ADD CONSTRAINT oauth_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.oauth_tokens
    ADD CONSTRAINT oauth_tokens_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY penfii.outbox
    ADD CONSTRAINT outbox_pkey PRIMARY KEY (event_id);

ALTER TABLE ONLY penfii.password_credentials
    ADD CONSTRAINT password_credentials_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY penfii.project_calendars
    ADD CONSTRAINT project_calendars_pkey PRIMARY KEY (project_id);

ALTER TABLE ONLY penfii.project_group_grants
    ADD CONSTRAINT project_group_grants_pkey PRIMARY KEY (project_id, group_id);

ALTER TABLE ONLY penfii.project_pins
    ADD CONSTRAINT project_pins_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.project_teams
    ADD CONSTRAINT project_teams_pkey PRIMARY KEY (project_id, team_id);

ALTER TABLE ONLY penfii.project_ticket_changes
    ADD CONSTRAINT project_ticket_changes_pkey PRIMARY KEY (project_id, change_seq);

ALTER TABLE ONLY penfii.project_ticket_order
    ADD CONSTRAINT project_ticket_order_pkey PRIMARY KEY (ticket_id);

ALTER TABLE ONLY penfii.project_user_grants
    ADD CONSTRAINT project_user_grants_pkey PRIMARY KEY (project_id, user_id);

ALTER TABLE ONLY penfii.project_views
    ADD CONSTRAINT project_views_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.projects
    ADD CONSTRAINT projects_id_workspace_id_key UNIQUE (id, workspace_id);

ALTER TABLE ONLY penfii.projects
    ADD CONSTRAINT projects_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.reminders
    ADD CONSTRAINT reminders_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.search_index_state
    ADD CONSTRAINT search_index_state_pkey PRIMARY KEY (kind);

ALTER TABLE ONLY penfii.sessions
    ADD CONSTRAINT sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.sessions
    ADD CONSTRAINT sessions_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY penfii.sprint_assignments
    ADD CONSTRAINT sprint_assignments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.sprint_baseline_tickets
    ADD CONSTRAINT sprint_baseline_tickets_pkey PRIMARY KEY (sprint_id, ticket_id);

ALTER TABLE ONLY penfii.sprint_baselines
    ADD CONSTRAINT sprint_baselines_pkey PRIMARY KEY (sprint_id);

ALTER TABLE ONLY penfii.sprints
    ADD CONSTRAINT sprints_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.subscriptions
    ADD CONSTRAINT subscriptions_pkey PRIMARY KEY (user_id, target_kind, target_id);

ALTER TABLE ONLY penfii.team_members
    ADD CONSTRAINT team_members_pkey PRIMARY KEY (team_id, user_id);

ALTER TABLE ONLY penfii.teams
    ADD CONSTRAINT teams_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.thread_reads
    ADD CONSTRAINT thread_reads_pkey PRIMARY KEY (thread_root_id, user_id);

ALTER TABLE ONLY penfii.ticket_attachments
    ADD CONSTRAINT ticket_attachments_file_id_key UNIQUE (file_id);

ALTER TABLE ONLY penfii.ticket_attachments
    ADD CONSTRAINT ticket_attachments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.ticket_comments
    ADD CONSTRAINT ticket_comments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_predecessor_id_successor_id_key UNIQUE (predecessor_id, successor_id);

ALTER TABLE ONLY penfii.ticket_labels
    ADD CONSTRAINT ticket_labels_pkey PRIMARY KEY (ticket_id, label_id);

ALTER TABLE ONLY penfii.ticket_milestones
    ADD CONSTRAINT ticket_milestones_pkey PRIMARY KEY (ticket_id, milestone_id);

ALTER TABLE ONLY penfii.ticket_revisions
    ADD CONSTRAINT ticket_revisions_pkey PRIMARY KEY (ticket_id, revision_no);

ALTER TABLE ONLY penfii.ticket_schedules
    ADD CONSTRAINT ticket_schedules_pkey PRIMARY KEY (ticket_id);

ALTER TABLE ONLY penfii.ticket_types
    ADD CONSTRAINT ticket_types_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_workspace_id_public_key_key UNIQUE (workspace_id, public_key);

ALTER TABLE ONLY penfii.users
    ADD CONSTRAINT users_email_key_key UNIQUE (email_key);

ALTER TABLE ONLY penfii.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.workflow_status_history
    ADD CONSTRAINT workflow_status_history_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.workflow_statuses
    ADD CONSTRAINT workflow_statuses_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.workflow_statuses
    ADD CONSTRAINT workflow_statuses_project_id_id_key UNIQUE (project_id, id);

ALTER TABLE ONLY penfii.workflow_transitions
    ADD CONSTRAINT workflow_transitions_pkey PRIMARY KEY (from_status_id, to_status_id);

ALTER TABLE ONLY penfii.workflows
    ADD CONSTRAINT workflows_pkey PRIMARY KEY (project_id);

ALTER TABLE ONLY penfii.workspace_auth_state
    ADD CONSTRAINT workspace_auth_state_pkey PRIMARY KEY (workspace_id);

ALTER TABLE ONLY penfii.workspace_graph_state
    ADD CONSTRAINT workspace_graph_state_pkey PRIMARY KEY (workspace_id);

ALTER TABLE ONLY penfii.workspace_invitations
    ADD CONSTRAINT workspace_invitations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY penfii.workspace_invitations
    ADD CONSTRAINT workspace_invitations_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY penfii.workspace_mcp_policy
    ADD CONSTRAINT workspace_mcp_policy_pkey PRIMARY KEY (workspace_id);

ALTER TABLE ONLY penfii.workspace_members
    ADD CONSTRAINT workspace_members_pkey PRIMARY KEY (workspace_id, user_id);

ALTER TABLE ONLY penfii.workspace_security_auditors
    ADD CONSTRAINT workspace_security_auditors_pkey PRIMARY KEY (workspace_id, user_id);

ALTER TABLE ONLY penfii.workspaces
    ADD CONSTRAINT workspaces_pkey PRIMARY KEY (id);

CREATE INDEX activity_ticket_idx ON penfii.activity_entries USING btree (ticket_id, occurred_at DESC, id DESC) WHERE (ticket_id IS NOT NULL);

CREATE INDEX audit_target_idx ON penfii.audit_entries USING btree (target_kind, target_id, occurred_at DESC, id DESC);

CREATE INDEX audit_workspace_time_idx ON penfii.audit_entries USING btree (workspace_id, occurred_at DESC, id DESC);

CREATE INDEX auth_attempts_created_idx ON penfii.auth_attempts USING btree (created_at);

CREATE UNIQUE INDEX auth_providers_key_uq ON penfii.auth_providers USING btree (provider_key);

CREATE INDEX channel_members_user_idx ON penfii.channel_members USING btree (user_id, channel_id);

CREATE INDEX channels_project_idx ON penfii.channels USING btree (project_id, id) WHERE (project_id IS NOT NULL);

CREATE INDEX channels_team_idx ON penfii.channels USING btree (team_id, id) WHERE (team_id IS NOT NULL);

CREATE INDEX consumer_receipts_work_idx ON penfii.consumer_receipts USING btree (consumer, available_at, event_id) WHERE (state = ANY (ARRAY['PENDING'::text, 'RUNNING'::text]));

CREATE INDEX conversations_workspace_idx ON penfii.conversations USING btree (workspace_id, id);

CREATE INDEX departments_workspace_idx ON penfii.departments USING btree (workspace_id, parent_id, "position", name);

CREATE INDEX dependency_waiver_history_idx ON penfii.dependency_waiver_history USING btree (dependency_id, created_at, id);

CREATE INDEX dm_members_user_idx ON penfii.dm_members USING btree (user_id, conversation_id);

CREATE INDEX doc_comment_files_comment_idx ON penfii.doc_comment_files USING btree (comment_id, attached_at, id);

CREATE INDEX doc_comments_page_idx ON penfii.doc_comments USING btree (page_id, created_at, id);

CREATE INDEX doc_page_files_file_idx ON penfii.doc_page_files USING btree (file_id);

CREATE INDEX doc_page_participants_user_idx ON penfii.doc_page_participants USING btree (user_id);

CREATE INDEX doc_page_targets_target_idx ON penfii.doc_page_targets USING btree (target_kind, target_id);

CREATE INDEX doc_pages_parent_idx ON penfii.doc_pages USING btree (parent_id, "position", id);

CREATE INDEX doc_pages_project_idx ON penfii.doc_pages USING btree (workspace_id, project_id) WHERE (deleted_at IS NULL);

CREATE INDEX doc_pages_workspace_idx ON penfii.doc_pages USING btree (workspace_id, id);

CREATE INDEX doc_templates_workspace_idx ON penfii.doc_templates USING btree (workspace_id, created_at, id);

CREATE INDEX doc_version_files_file_idx ON penfii.doc_version_files USING btree (page_id, file_id);

CREATE UNIQUE INDEX domain_events_seq_idx ON penfii.domain_events USING btree (seq);

CREATE INDEX domain_events_target_idx ON penfii.domain_events USING btree (target_kind, target_id, seq);

CREATE UNIQUE INDEX external_identities_id_uq ON penfii.external_identities USING btree (id);

CREATE INDEX external_identities_user_idx ON penfii.external_identities USING btree (user_id);

CREATE UNIQUE INDEX external_identities_user_provider_uq ON penfii.external_identities USING btree (user_id, provider_id);

CREATE UNIQUE INDEX favorite_folders_name_idx ON penfii.favorite_folders USING btree (user_id, workspace_id, lower(name));

CREATE INDEX favorite_folders_user_idx ON penfii.favorite_folders USING btree (user_id, workspace_id, "position", id);

CREATE INDEX favorites_folder_idx ON penfii.favorites USING btree (folder_id) WHERE (folder_id IS NOT NULL);

CREATE INDEX favorites_user_idx ON penfii.favorites USING btree (user_id, workspace_id, "position", target_id);

CREATE INDEX file_tasks_lease_idx ON penfii.file_processing_tasks USING btree (lease_until, id) WHERE (state = 'RUNNING'::text);

CREATE INDEX file_tasks_ready_idx ON penfii.file_processing_tasks USING btree (task_kind, available_at, id) WHERE (state = 'READY'::text);

CREATE INDEX groups_workspace_idx ON penfii.groups USING btree (workspace_id, id);

CREATE INDEX inbox_items_list_idx ON penfii.inbox_items USING btree (recipient_id, workspace_id, created_at DESC, id DESC) WHERE (state <> 'ARCHIVED'::text);

CREATE UNIQUE INDEX later_items_active_idx ON penfii.later_items USING btree (user_id, kind, target_id) WHERE (state = 'ACTIVE'::text);

CREATE INDEX later_items_user_idx ON penfii.later_items USING btree (user_id, workspace_id, state, created_at, id);

CREATE INDEX mcp_call_log_at_idx ON penfii.mcp_call_log USING btree (at);

CREATE INDEX mcp_call_log_resource_idx ON penfii.mcp_call_log USING btree (resource_type, resource_id, resource_revision) WHERE ((outcome = 'ok'::text) AND (resource_id IS NOT NULL));

CREATE INDEX mcp_call_log_user_idx ON penfii.mcp_call_log USING btree (user_id, at DESC);

CREATE INDEX mcp_call_log_workspace_idx ON penfii.mcp_call_log USING btree (workspace_id, at DESC);

CREATE INDEX message_attachments_file_idx ON penfii.message_attachments USING btree (file_id);

CREATE INDEX message_version_files_file_idx ON penfii.message_version_files USING btree (file_id);

CREATE INDEX messages_roots_idx ON penfii.messages USING btree (conversation_id, stream_seq) WHERE (thread_root_id IS NULL);

CREATE INDEX messages_thread_idx ON penfii.messages USING btree (thread_root_id, stream_seq) WHERE (thread_root_id IS NOT NULL);

CREATE INDEX milestones_project_idx ON penfii.milestones USING btree (project_id, created_at, id);

CREATE UNIQUE INDEX oauth_grants_live_uq ON penfii.oauth_grants USING btree (user_id, client_pk) WHERE (revoked_at IS NULL);

CREATE INDEX oauth_tokens_expires_idx ON penfii.oauth_tokens USING btree (expires_at);

CREATE INDEX oauth_tokens_family_idx ON penfii.oauth_tokens USING btree (family_id);

CREATE INDEX oauth_tokens_grant_idx ON penfii.oauth_tokens USING btree (grant_id);

CREATE INDEX outbox_pending_idx ON penfii.outbox USING btree (available_at, event_id) WHERE (dispatched_at IS NULL);

CREATE INDEX project_pins_project_idx ON penfii.project_pins USING btree (project_id, "position", id);

CREATE UNIQUE INDEX project_pins_ticket_idx ON penfii.project_pins USING btree (project_id, ticket_id) WHERE (ticket_id IS NOT NULL);

CREATE UNIQUE INDEX project_pins_url_idx ON penfii.project_pins USING btree (project_id, url) WHERE (url IS NOT NULL);

CREATE INDEX project_teams_team_idx ON penfii.project_teams USING btree (team_id, project_id);

CREATE INDEX project_ticket_order_scan_idx ON penfii.project_ticket_order USING btree (project_id, sort_key, ticket_id);

CREATE UNIQUE INDEX project_views_name_idx ON penfii.project_views USING btree (project_id, scope, (
CASE
    WHEN (scope = 'PERSONAL'::text) THEN owner_id
    ELSE '00000000-0000-0000-0000-000000000000'::uuid
END), lower(name)) WHERE (deleted_at IS NULL);

CREATE INDEX project_views_project_idx ON penfii.project_views USING btree (project_id, created_at, id) WHERE (deleted_at IS NULL);

CREATE INDEX project_views_wide_idx ON penfii.project_views USING btree (workspace_id, owner_id, target_kind, created_at, id) WHERE ((project_id IS NULL) AND (deleted_at IS NULL));

CREATE UNIQUE INDEX project_views_wide_name_idx ON penfii.project_views USING btree (workspace_id, target_kind, COALESCE(team_id, '00000000-0000-0000-0000-000000000000'::uuid), owner_id, lower(name)) WHERE ((project_id IS NULL) AND (deleted_at IS NULL));

CREATE UNIQUE INDEX projects_ticket_prefix_ci_idx ON penfii.projects USING btree (workspace_id, lower(ticket_prefix)) WHERE (ticket_prefix IS NOT NULL);

CREATE INDEX projects_workspace_idx ON penfii.projects USING btree (workspace_id, id);

CREATE INDEX reminders_due_idx ON penfii.reminders USING btree (due_at, id) WHERE (state = 'SCHEDULED'::text);

CREATE INDEX reminders_later_idx ON penfii.reminders USING btree (later_id, created_at DESC);

CREATE UNIQUE INDEX reminders_one_scheduled_idx ON penfii.reminders USING btree (later_id) WHERE (state = 'SCHEDULED'::text);

CREATE INDEX sessions_user_live_idx ON penfii.sessions USING btree (user_id, created_at DESC, id) WHERE (revoked_at IS NULL);

CREATE INDEX sprint_assignments_history_idx ON penfii.sprint_assignments USING btree (sprint_id, ticket_id, assigned_at);

CREATE UNIQUE INDEX sprint_one_current_assignment_idx ON penfii.sprint_assignments USING btree (ticket_id) WHERE (ended_at IS NULL);

CREATE INDEX sprints_project_state_idx ON penfii.sprints USING btree (project_id, state, id);

CREATE INDEX subscriptions_target_idx ON penfii.subscriptions USING btree (target_kind, target_id, user_id);

CREATE INDEX subscriptions_user_idx ON penfii.subscriptions USING btree (user_id, workspace_id, target_kind);

CREATE INDEX team_members_user_idx ON penfii.team_members USING btree (user_id, team_id);

CREATE INDEX teams_workspace_idx ON penfii.teams USING btree (workspace_id, id);

CREATE INDEX ticket_attachments_comment_idx ON penfii.ticket_attachments USING btree (comment_id) WHERE (comment_id IS NOT NULL);

CREATE INDEX ticket_attachments_ticket_idx ON penfii.ticket_attachments USING btree (ticket_id, attached_at, id);

CREATE INDEX ticket_comments_timeline_idx ON penfii.ticket_comments USING btree (ticket_id, created_at, id);

CREATE INDEX ticket_dependencies_project_idx ON penfii.ticket_dependencies USING btree (project_id);

CREATE INDEX ticket_dependencies_successor_idx ON penfii.ticket_dependencies USING btree (successor_id, predecessor_id);

CREATE INDEX ticket_milestones_milestone_idx ON penfii.ticket_milestones USING btree (milestone_id, ticket_id);

CREATE UNIQUE INDEX ticket_related_pair_idx ON penfii.ticket_dependencies USING btree (LEAST(predecessor_id, successor_id), GREATEST(predecessor_id, successor_id)) WHERE (kind = 'RELATED'::text);

CREATE INDEX tickets_parent_live_idx ON penfii.tickets USING btree (parent_id) WHERE ((parent_id IS NOT NULL) AND (archived_at IS NULL) AND (trashed_at IS NULL) AND (purged_at IS NULL));

CREATE UNIQUE INDEX tickets_project_sequence_idx ON penfii.tickets USING btree (project_id, project_sequence) WHERE (project_sequence IS NOT NULL);

CREATE INDEX tickets_project_state_live_idx ON penfii.tickets USING btree (project_id, status_id, created_at, id) WHERE ((archived_at IS NULL) AND (trashed_at IS NULL) AND (purged_at IS NULL));

CREATE INDEX tickets_trash_idx ON penfii.tickets USING btree (project_id, trashed_at DESC, id) WHERE ((trashed_at IS NOT NULL) AND (purged_at IS NULL));

CREATE UNIQUE INDEX users_username_key_idx ON penfii.users USING btree (username_key) WHERE (username_key IS NOT NULL);

CREATE UNIQUE INDEX workflow_one_initial_idx ON penfii.workflow_statuses USING btree (project_id) WHERE is_initial;

CREATE INDEX workflow_status_history_status_idx ON penfii.workflow_status_history USING btree (status_id, effective_from, id);

CREATE INDEX workspace_invitations_workspace_idx ON penfii.workspace_invitations USING btree (workspace_id, created_at DESC, id DESC);

CREATE INDEX workspace_members_user_active_idx ON penfii.workspace_members USING btree (user_id, workspace_id) WHERE (state = 'ACTIVE'::text);

CREATE INDEX workspace_members_workspace_idx ON penfii.workspace_members USING btree (workspace_id, state, role);

CREATE TRIGGER doc_pages_title_required BEFORE INSERT OR UPDATE OF title ON penfii.doc_pages FOR EACH ROW EXECUTE FUNCTION penfii.doc_pages_title_required();

CREATE TRIGGER doc_versions_immutable BEFORE DELETE OR UPDATE ON penfii.doc_versions FOR EACH ROW EXECUTE FUNCTION penfii.doc_versions_immutable();

ALTER TABLE ONLY penfii.activity_entries
    ADD CONSTRAINT activity_entries_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.activity_entries
    ADD CONSTRAINT activity_entries_origin_channel_id_fkey FOREIGN KEY (origin_channel_id) REFERENCES penfii.channels(id);

ALTER TABLE ONLY penfii.activity_entries
    ADD CONSTRAINT activity_entries_origin_project_id_fkey FOREIGN KEY (origin_project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.activity_entries
    ADD CONSTRAINT activity_entries_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.activity_entries
    ADD CONSTRAINT activity_entries_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.audit_entries
    ADD CONSTRAINT audit_entries_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.audit_entries
    ADD CONSTRAINT audit_entries_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.auth_attempts
    ADD CONSTRAINT auth_attempts_invitation_id_fkey FOREIGN KEY (invitation_id) REFERENCES penfii.workspace_invitations(id);

ALTER TABLE ONLY penfii.auth_attempts
    ADD CONSTRAINT auth_attempts_provider_id_fkey FOREIGN KEY (provider_id) REFERENCES penfii.auth_providers(id);

ALTER TABLE ONLY penfii.auth_attempts
    ADD CONSTRAINT auth_attempts_session_id_fkey FOREIGN KEY (session_id) REFERENCES penfii.sessions(id);

ALTER TABLE ONLY penfii.auth_attempts
    ADD CONSTRAINT auth_attempts_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.channel_content_auditors
    ADD CONSTRAINT channel_content_auditors_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES penfii.channels(id);

ALTER TABLE ONLY penfii.channel_content_auditors
    ADD CONSTRAINT channel_content_auditors_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.channel_group_grants
    ADD CONSTRAINT channel_group_grants_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES penfii.channels(id);

ALTER TABLE ONLY penfii.channel_group_grants
    ADD CONSTRAINT channel_group_grants_group_id_fkey FOREIGN KEY (group_id) REFERENCES penfii.groups(id);

ALTER TABLE ONLY penfii.channel_members
    ADD CONSTRAINT channel_members_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES penfii.channels(id);

ALTER TABLE ONLY penfii.channel_members
    ADD CONSTRAINT channel_members_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.channel_mutes
    ADD CONSTRAINT channel_mutes_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES penfii.channels(id);

ALTER TABLE ONLY penfii.channel_mutes
    ADD CONSTRAINT channel_mutes_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.channels
    ADD CONSTRAINT channels_id_fkey FOREIGN KEY (id) REFERENCES penfii.conversations(id);

ALTER TABLE ONLY penfii.channels
    ADD CONSTRAINT channels_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.channels
    ADD CONSTRAINT channels_team_id_fkey FOREIGN KEY (team_id) REFERENCES penfii.teams(id);

ALTER TABLE ONLY penfii.commands
    ADD CONSTRAINT commands_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.consumer_receipts
    ADD CONSTRAINT consumer_receipts_event_id_fkey FOREIGN KEY (event_id) REFERENCES penfii.domain_events(event_id);

ALTER TABLE ONLY penfii.conversation_changes
    ADD CONSTRAINT conversation_changes_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES penfii.conversations(id);

ALTER TABLE ONLY penfii.conversation_changes
    ADD CONSTRAINT conversation_changes_message_id_fkey FOREIGN KEY (message_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.conversation_reads
    ADD CONSTRAINT conversation_reads_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES penfii.conversations(id);

ALTER TABLE ONLY penfii.conversation_reads
    ADD CONSTRAINT conversation_reads_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.conversations
    ADD CONSTRAINT conversations_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.departments
    ADD CONSTRAINT departments_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES penfii.departments(id);

ALTER TABLE ONLY penfii.departments
    ADD CONSTRAINT departments_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.dependency_waiver_history
    ADD CONSTRAINT dependency_waiver_history_actor_id_fkey FOREIGN KEY (actor_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.dependency_waiver_history
    ADD CONSTRAINT dependency_waiver_history_predecessor_id_fkey FOREIGN KEY (predecessor_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.dependency_waiver_history
    ADD CONSTRAINT dependency_waiver_history_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.dependency_waiver_history
    ADD CONSTRAINT dependency_waiver_history_successor_id_fkey FOREIGN KEY (successor_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.dependency_waiver_history
    ADD CONSTRAINT dependency_waiver_history_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.dm_members
    ADD CONSTRAINT dm_members_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES penfii.conversations(id);

ALTER TABLE ONLY penfii.dm_members
    ADD CONSTRAINT dm_members_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_collab_state
    ADD CONSTRAINT doc_collab_state_page_id_fkey FOREIGN KEY (page_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.doc_collab_updates
    ADD CONSTRAINT doc_collab_updates_page_id_fkey FOREIGN KEY (page_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.doc_comment_files
    ADD CONSTRAINT doc_comment_files_comment_id_fkey FOREIGN KEY (comment_id) REFERENCES penfii.doc_comments(id) ON DELETE CASCADE;

ALTER TABLE ONLY penfii.doc_comment_files
    ADD CONSTRAINT doc_comment_files_file_id_fkey FOREIGN KEY (file_id) REFERENCES penfii.files(id);

ALTER TABLE ONLY penfii.doc_comments
    ADD CONSTRAINT doc_comments_author_id_fkey FOREIGN KEY (author_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_comments
    ADD CONSTRAINT doc_comments_page_id_fkey FOREIGN KEY (page_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.doc_page_files
    ADD CONSTRAINT doc_page_files_attached_by_fkey FOREIGN KEY (attached_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_page_files
    ADD CONSTRAINT doc_page_files_file_id_fkey FOREIGN KEY (file_id) REFERENCES penfii.files(id);

ALTER TABLE ONLY penfii.doc_page_files
    ADD CONSTRAINT doc_page_files_page_id_fkey FOREIGN KEY (page_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.doc_page_participants
    ADD CONSTRAINT doc_page_participants_page_id_fkey FOREIGN KEY (page_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.doc_page_participants
    ADD CONSTRAINT doc_page_participants_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_page_targets
    ADD CONSTRAINT doc_page_targets_page_id_fkey FOREIGN KEY (page_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_scope_project_id_fkey FOREIGN KEY (scope_project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_scope_team_id_fkey FOREIGN KEY (scope_team_id) REFERENCES penfii.teams(id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_pages
    ADD CONSTRAINT doc_pages_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.doc_templates
    ADD CONSTRAINT doc_templates_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_templates
    ADD CONSTRAINT doc_templates_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.doc_version_files
    ADD CONSTRAINT doc_version_files_file_id_fkey FOREIGN KEY (file_id) REFERENCES penfii.files(id);

ALTER TABLE ONLY penfii.doc_version_files
    ADD CONSTRAINT doc_version_files_page_id_version_no_fkey FOREIGN KEY (page_id, version_no) REFERENCES penfii.doc_versions(page_id, version_no);

ALTER TABLE ONLY penfii.doc_versions
    ADD CONSTRAINT doc_versions_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.doc_versions
    ADD CONSTRAINT doc_versions_page_id_fkey FOREIGN KEY (page_id) REFERENCES penfii.doc_pages(id);

ALTER TABLE ONLY penfii.domain_events
    ADD CONSTRAINT domain_events_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.domain_events
    ADD CONSTRAINT domain_events_initiated_by_user_id_fkey FOREIGN KEY (initiated_by_user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.domain_events
    ADD CONSTRAINT domain_events_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.draft_conflicts
    ADD CONSTRAINT draft_conflicts_draft_id_fkey FOREIGN KEY (draft_id) REFERENCES penfii.drafts(id);

ALTER TABLE ONLY penfii.drafts
    ADD CONSTRAINT drafts_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.drafts
    ADD CONSTRAINT drafts_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.event_recipients
    ADD CONSTRAINT event_recipients_event_id_fkey FOREIGN KEY (event_id) REFERENCES penfii.domain_events(event_id);

ALTER TABLE ONLY penfii.event_recipients
    ADD CONSTRAINT event_recipients_recipient_id_fkey FOREIGN KEY (recipient_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.external_identities
    ADD CONSTRAINT external_identities_provider_id_fkey FOREIGN KEY (provider_id) REFERENCES penfii.auth_providers(id);

ALTER TABLE ONLY penfii.external_identities
    ADD CONSTRAINT external_identities_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.favorite_folders
    ADD CONSTRAINT favorite_folders_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.favorite_folders
    ADD CONSTRAINT favorite_folders_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.favorites
    ADD CONSTRAINT favorites_folder_id_fkey FOREIGN KEY (folder_id) REFERENCES penfii.favorite_folders(id) ON DELETE SET NULL;

ALTER TABLE ONLY penfii.favorites
    ADD CONSTRAINT favorites_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.favorites
    ADD CONSTRAINT favorites_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.file_processing_results
    ADD CONSTRAINT file_processing_results_task_id_fkey FOREIGN KEY (task_id) REFERENCES penfii.file_processing_tasks(id);

ALTER TABLE ONLY penfii.file_processing_tasks
    ADD CONSTRAINT file_processing_tasks_file_id_fkey FOREIGN KEY (file_id) REFERENCES penfii.files(id);

ALTER TABLE ONLY penfii.files
    ADD CONSTRAINT files_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.files
    ADD CONSTRAINT files_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.group_members
    ADD CONSTRAINT group_members_group_id_fkey FOREIGN KEY (group_id) REFERENCES penfii.groups(id);

ALTER TABLE ONLY penfii.group_members
    ADD CONSTRAINT group_members_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.groups
    ADD CONSTRAINT groups_system_team_id_fkey FOREIGN KEY (system_team_id) REFERENCES penfii.teams(id);

ALTER TABLE ONLY penfii.groups
    ADD CONSTRAINT groups_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.inbox_items
    ADD CONSTRAINT inbox_items_event_id_recipient_id_notification_kind_fkey FOREIGN KEY (event_id, recipient_id, notification_kind) REFERENCES penfii.event_recipients(event_id, recipient_id, notification_kind);

ALTER TABLE ONLY penfii.inbox_items
    ADD CONSTRAINT inbox_items_recipient_id_fkey FOREIGN KEY (recipient_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.inbox_items
    ADD CONSTRAINT inbox_items_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.labels
    ADD CONSTRAINT labels_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.later_items
    ADD CONSTRAINT later_items_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.later_items
    ADD CONSTRAINT later_items_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.mcp_call_log
    ADD CONSTRAINT mcp_call_log_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.message_attachments
    ADD CONSTRAINT message_attachments_file_id_fkey FOREIGN KEY (file_id) REFERENCES penfii.files(id);

ALTER TABLE ONLY penfii.message_attachments
    ADD CONSTRAINT message_attachments_message_id_fkey FOREIGN KEY (message_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.message_mentions
    ADD CONSTRAINT message_mentions_group_id_fkey FOREIGN KEY (group_id) REFERENCES penfii.groups(id);

ALTER TABLE ONLY penfii.message_mentions
    ADD CONSTRAINT message_mentions_message_id_fkey FOREIGN KEY (message_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.message_mentions
    ADD CONSTRAINT message_mentions_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.message_reactions
    ADD CONSTRAINT message_reactions_message_id_fkey FOREIGN KEY (message_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.message_reactions
    ADD CONSTRAINT message_reactions_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.message_version_files
    ADD CONSTRAINT message_version_files_file_id_fkey FOREIGN KEY (file_id) REFERENCES penfii.files(id);

ALTER TABLE ONLY penfii.message_version_files
    ADD CONSTRAINT message_version_files_message_id_version_fkey FOREIGN KEY (message_id, version) REFERENCES penfii.message_versions(message_id, version);

ALTER TABLE ONLY penfii.message_versions
    ADD CONSTRAINT message_versions_changed_by_fkey FOREIGN KEY (changed_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.message_versions
    ADD CONSTRAINT message_versions_message_id_fkey FOREIGN KEY (message_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.messages
    ADD CONSTRAINT messages_author_id_fkey FOREIGN KEY (author_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.messages
    ADD CONSTRAINT messages_conversation_id_fkey FOREIGN KEY (conversation_id) REFERENCES penfii.conversations(id);

ALTER TABLE ONLY penfii.messages
    ADD CONSTRAINT messages_reply_to_id_fkey FOREIGN KEY (reply_to_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.messages
    ADD CONSTRAINT messages_thread_root_id_fkey FOREIGN KEY (thread_root_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.milestones
    ADD CONSTRAINT milestones_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.milestones
    ADD CONSTRAINT milestones_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.notification_preferences
    ADD CONSTRAINT notification_preferences_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.oauth_authorization_requests
    ADD CONSTRAINT oauth_authorization_requests_client_pk_fkey FOREIGN KEY (client_pk) REFERENCES penfii.oauth_clients(id);

ALTER TABLE ONLY penfii.oauth_authorization_requests
    ADD CONSTRAINT oauth_authorization_requests_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.oauth_codes
    ADD CONSTRAINT oauth_codes_client_pk_fkey FOREIGN KEY (client_pk) REFERENCES penfii.oauth_clients(id);

ALTER TABLE ONLY penfii.oauth_codes
    ADD CONSTRAINT oauth_codes_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES penfii.oauth_grants(id);

ALTER TABLE ONLY penfii.oauth_grants
    ADD CONSTRAINT oauth_grants_client_pk_fkey FOREIGN KEY (client_pk) REFERENCES penfii.oauth_clients(id);

ALTER TABLE ONLY penfii.oauth_grants
    ADD CONSTRAINT oauth_grants_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.oauth_tokens
    ADD CONSTRAINT oauth_tokens_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES penfii.oauth_grants(id);

ALTER TABLE ONLY penfii.outbox
    ADD CONSTRAINT outbox_event_id_fkey FOREIGN KEY (event_id) REFERENCES penfii.domain_events(event_id);

ALTER TABLE ONLY penfii.password_credentials
    ADD CONSTRAINT password_credentials_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.project_calendars
    ADD CONSTRAINT project_calendars_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_calendars
    ADD CONSTRAINT project_calendars_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.project_group_grants
    ADD CONSTRAINT project_group_grants_group_id_fkey FOREIGN KEY (group_id) REFERENCES penfii.groups(id);

ALTER TABLE ONLY penfii.project_group_grants
    ADD CONSTRAINT project_group_grants_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_pins
    ADD CONSTRAINT project_pins_pinned_by_fkey FOREIGN KEY (pinned_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.project_pins
    ADD CONSTRAINT project_pins_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_pins
    ADD CONSTRAINT project_pins_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.project_pins
    ADD CONSTRAINT project_pins_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.project_teams
    ADD CONSTRAINT project_teams_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_teams
    ADD CONSTRAINT project_teams_team_id_fkey FOREIGN KEY (team_id) REFERENCES penfii.teams(id);

ALTER TABLE ONLY penfii.project_ticket_changes
    ADD CONSTRAINT project_ticket_changes_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_ticket_changes
    ADD CONSTRAINT project_ticket_changes_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.project_ticket_order
    ADD CONSTRAINT project_ticket_order_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_ticket_order
    ADD CONSTRAINT project_ticket_order_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.project_user_grants
    ADD CONSTRAINT project_user_grants_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_user_grants
    ADD CONSTRAINT project_user_grants_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.project_views
    ADD CONSTRAINT project_views_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.project_views
    ADD CONSTRAINT project_views_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.project_views
    ADD CONSTRAINT project_views_team_id_fkey FOREIGN KEY (team_id) REFERENCES penfii.teams(id);

ALTER TABLE ONLY penfii.project_views
    ADD CONSTRAINT project_views_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.projects
    ADD CONSTRAINT projects_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.reminders
    ADD CONSTRAINT reminders_later_id_fkey FOREIGN KEY (later_id) REFERENCES penfii.later_items(id) ON DELETE CASCADE;

ALTER TABLE ONLY penfii.reminders
    ADD CONSTRAINT reminders_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.reminders
    ADD CONSTRAINT reminders_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.sessions
    ADD CONSTRAINT sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.sprint_assignments
    ADD CONSTRAINT sprint_assignments_assigned_by_fkey FOREIGN KEY (assigned_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.sprint_assignments
    ADD CONSTRAINT sprint_assignments_carried_from_sprint_id_fkey FOREIGN KEY (carried_from_sprint_id) REFERENCES penfii.sprints(id);

ALTER TABLE ONLY penfii.sprint_assignments
    ADD CONSTRAINT sprint_assignments_carried_to_sprint_id_fkey FOREIGN KEY (carried_to_sprint_id) REFERENCES penfii.sprints(id);

ALTER TABLE ONLY penfii.sprint_assignments
    ADD CONSTRAINT sprint_assignments_sprint_id_fkey FOREIGN KEY (sprint_id) REFERENCES penfii.sprints(id);

ALTER TABLE ONLY penfii.sprint_assignments
    ADD CONSTRAINT sprint_assignments_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.sprint_baseline_tickets
    ADD CONSTRAINT sprint_baseline_tickets_sprint_id_fkey FOREIGN KEY (sprint_id) REFERENCES penfii.sprints(id);

ALTER TABLE ONLY penfii.sprint_baseline_tickets
    ADD CONSTRAINT sprint_baseline_tickets_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.sprint_baselines
    ADD CONSTRAINT sprint_baselines_recorded_by_fkey FOREIGN KEY (recorded_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.sprint_baselines
    ADD CONSTRAINT sprint_baselines_sprint_id_fkey FOREIGN KEY (sprint_id) REFERENCES penfii.sprints(id);

ALTER TABLE ONLY penfii.sprints
    ADD CONSTRAINT sprints_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.sprints
    ADD CONSTRAINT sprints_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.subscriptions
    ADD CONSTRAINT subscriptions_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.subscriptions
    ADD CONSTRAINT subscriptions_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.team_members
    ADD CONSTRAINT team_members_team_id_fkey FOREIGN KEY (team_id) REFERENCES penfii.teams(id);

ALTER TABLE ONLY penfii.team_members
    ADD CONSTRAINT team_members_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.teams
    ADD CONSTRAINT teams_department_id_fkey FOREIGN KEY (department_id) REFERENCES penfii.departments(id);

ALTER TABLE ONLY penfii.teams
    ADD CONSTRAINT teams_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.thread_reads
    ADD CONSTRAINT thread_reads_thread_root_id_fkey FOREIGN KEY (thread_root_id) REFERENCES penfii.messages(id);

ALTER TABLE ONLY penfii.thread_reads
    ADD CONSTRAINT thread_reads_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_attachments
    ADD CONSTRAINT ticket_attachments_attached_by_fkey FOREIGN KEY (attached_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_attachments
    ADD CONSTRAINT ticket_attachments_comment_id_fkey FOREIGN KEY (comment_id) REFERENCES penfii.ticket_comments(id);

ALTER TABLE ONLY penfii.ticket_attachments
    ADD CONSTRAINT ticket_attachments_file_id_fkey FOREIGN KEY (file_id) REFERENCES penfii.files(id);

ALTER TABLE ONLY penfii.ticket_attachments
    ADD CONSTRAINT ticket_attachments_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_comments
    ADD CONSTRAINT ticket_comments_author_id_fkey FOREIGN KEY (author_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_comments
    ADD CONSTRAINT ticket_comments_reply_to_id_fkey FOREIGN KEY (reply_to_id) REFERENCES penfii.ticket_comments(id);

ALTER TABLE ONLY penfii.ticket_comments
    ADD CONSTRAINT ticket_comments_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_predecessor_id_fkey FOREIGN KEY (predecessor_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_successor_id_fkey FOREIGN KEY (successor_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_waived_by_fkey FOREIGN KEY (waived_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_dependencies
    ADD CONSTRAINT ticket_dependencies_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.ticket_labels
    ADD CONSTRAINT ticket_labels_label_id_fkey FOREIGN KEY (label_id) REFERENCES penfii.labels(id);

ALTER TABLE ONLY penfii.ticket_labels
    ADD CONSTRAINT ticket_labels_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_milestones
    ADD CONSTRAINT ticket_milestones_linked_by_fkey FOREIGN KEY (linked_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_milestones
    ADD CONSTRAINT ticket_milestones_milestone_id_fkey FOREIGN KEY (milestone_id) REFERENCES penfii.milestones(id);

ALTER TABLE ONLY penfii.ticket_milestones
    ADD CONSTRAINT ticket_milestones_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_revisions
    ADD CONSTRAINT ticket_revisions_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_revisions
    ADD CONSTRAINT ticket_revisions_initiated_by_user_id_fkey FOREIGN KEY (initiated_by_user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.ticket_revisions
    ADD CONSTRAINT ticket_revisions_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_schedules
    ADD CONSTRAINT ticket_schedules_ticket_id_fkey FOREIGN KEY (ticket_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.ticket_types
    ADD CONSTRAINT ticket_types_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES penfii.channels(id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES penfii.tickets(id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_project_id_status_id_fkey FOREIGN KEY (project_id, status_id) REFERENCES penfii.workflow_statuses(project_id, id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_project_id_workspace_id_fkey FOREIGN KEY (project_id, workspace_id) REFERENCES penfii.projects(id, workspace_id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_purged_by_fkey FOREIGN KEY (purged_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_ticket_type_id_fkey FOREIGN KEY (ticket_type_id) REFERENCES penfii.ticket_types(id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_trashed_by_fkey FOREIGN KEY (trashed_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_workspace_id_assignee_id_fkey FOREIGN KEY (workspace_id, assignee_id) REFERENCES penfii.workspace_members(workspace_id, user_id);

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.workflow_status_history
    ADD CONSTRAINT workflow_status_history_status_id_fkey FOREIGN KEY (status_id) REFERENCES penfii.workflow_statuses(id);

ALTER TABLE ONLY penfii.workflow_statuses
    ADD CONSTRAINT workflow_statuses_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.workflows(project_id);

ALTER TABLE ONLY penfii.workflow_transitions
    ADD CONSTRAINT workflow_transitions_from_status_id_fkey FOREIGN KEY (from_status_id) REFERENCES penfii.workflow_statuses(id);

ALTER TABLE ONLY penfii.workflow_transitions
    ADD CONSTRAINT workflow_transitions_to_status_id_fkey FOREIGN KEY (to_status_id) REFERENCES penfii.workflow_statuses(id);

ALTER TABLE ONLY penfii.workflows
    ADD CONSTRAINT workflows_project_id_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id);

ALTER TABLE ONLY penfii.workspace_auth_state
    ADD CONSTRAINT workspace_auth_state_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.workspace_graph_state
    ADD CONSTRAINT workspace_graph_state_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.workspace_invitations
    ADD CONSTRAINT workspace_invitations_accepted_user_id_fkey FOREIGN KEY (accepted_user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.workspace_invitations
    ADD CONSTRAINT workspace_invitations_created_by_fkey FOREIGN KEY (created_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.workspace_invitations
    ADD CONSTRAINT workspace_invitations_team_id_fkey FOREIGN KEY (team_id) REFERENCES penfii.teams(id) ON DELETE SET NULL;

ALTER TABLE ONLY penfii.workspace_invitations
    ADD CONSTRAINT workspace_invitations_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.workspace_mcp_policy
    ADD CONSTRAINT workspace_mcp_policy_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.workspace_mcp_policy
    ADD CONSTRAINT workspace_mcp_policy_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.workspace_members
    ADD CONSTRAINT workspace_members_user_id_fkey FOREIGN KEY (user_id) REFERENCES penfii.users(id);

ALTER TABLE ONLY penfii.workspace_members
    ADD CONSTRAINT workspace_members_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES penfii.workspaces(id);

ALTER TABLE ONLY penfii.workspace_security_auditors
    ADD CONSTRAINT workspace_security_auditors_workspace_id_user_id_fkey FOREIGN KEY (workspace_id, user_id) REFERENCES penfii.workspace_members(workspace_id, user_id);

-- Rows every instance starts with.
INSERT INTO penfii.foundation_state (key) VALUES ('outbox_cutover');
INSERT INTO penfii.instance_settings (singleton, default_locale) VALUES (true, 'ko-KR');

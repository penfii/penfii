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

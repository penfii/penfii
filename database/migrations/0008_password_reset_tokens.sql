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

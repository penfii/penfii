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

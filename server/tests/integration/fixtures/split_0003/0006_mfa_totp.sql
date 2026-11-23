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

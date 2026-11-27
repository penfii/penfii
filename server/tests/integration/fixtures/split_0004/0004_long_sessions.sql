-- FEA-182: a person can choose to stay signed in. Such a "long" session has an absolute end (a fixed
-- time after it was issued, never moved) and a sliding inactivity window (moved by activity, never
-- past the absolute end). `expires_at` stays the one time every check reads (`expires_at > now()`):
-- for a long session it is the earlier of the two ends, and activity only moves it within the cap. A
-- session without `absolute_expires_at` is an ordinary session and behaves exactly as before; its
-- `expires_at` is fixed when it is issued.
--
-- `idle_seconds` is the inactivity window the session was issued with (or was shortened to by the
-- operator, see `identity::apply_long_session_policy`). Both columns are set together or not at all.

ALTER TABLE penfii.sessions
    ADD COLUMN absolute_expires_at timestamp with time zone,
    ADD COLUMN idle_seconds integer,
    ADD CONSTRAINT sessions_long_chk CHECK (
        (absolute_expires_at IS NULL AND idle_seconds IS NULL)
        OR (absolute_expires_at IS NOT NULL AND idle_seconds IS NOT NULL AND idle_seconds > 0)
    );

-- The choice made on the sign-in page, kept with the OAuth/OIDC attempt that it belongs to. Only a
-- `login` attempt can carry it; the callback reads it from here, never from the query string.
ALTER TABLE penfii.auth_attempts
    ADD COLUMN keep_signed_in boolean DEFAULT false NOT NULL,
    ADD CONSTRAINT auth_attempts_keep_signed_in_chk CHECK ((NOT keep_signed_in) OR intent = 'login');

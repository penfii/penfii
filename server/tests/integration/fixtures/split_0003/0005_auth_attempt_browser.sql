-- FEA-143: an external sign-in attempt belongs to the browser that started it. The start sets a short-lived
-- cookie whose random value is kept here as a hash; the callback only consumes an attempt whose cookie
-- it can show. An attempt started before this column existed has no hash and is not consumed (it lives
-- ten minutes at most; the person signs in again).

ALTER TABLE penfii.auth_attempts ADD COLUMN browser_hash bytea;

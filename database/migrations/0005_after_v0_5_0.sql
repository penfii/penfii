-- Every schema and data change after v0.5.0, as one file.

-- ===== favorites of DMs and Docs =====

-- FEA-202, FEA-203: a Favorite can also point at a DM (the conversation id) and at a Doc page. The
-- check only grows: every row it allowed before is still allowed, so no row is converted.

ALTER TABLE penfii.favorites DROP CONSTRAINT favorites_kind_check;
ALTER TABLE penfii.favorites ADD CONSTRAINT favorites_kind_check CHECK (
    kind = ANY (ARRAY['PROJECT', 'TEAM', 'TICKET', 'VIEW', 'CHANNEL', 'DM', 'DOC'])
);

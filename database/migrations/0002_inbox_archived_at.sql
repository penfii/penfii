-- Inbox: when an item was archived. Written by the `archive` command, cleared when the item leaves
-- the archive. Items archived before this column existed have no value (it cannot be recovered);
-- the archive lists them by `created_at` instead.

ALTER TABLE penfii.inbox_items ADD COLUMN archived_at timestamp with time zone;

CREATE INDEX inbox_items_archived_idx ON penfii.inbox_items USING btree (recipient_id, workspace_id, (COALESCE(archived_at, created_at)) DESC, id DESC) WHERE (state = 'ARCHIVED'::text);

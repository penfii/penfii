-- A person who no longer takes part in a Project has no membership in its channels, and getting the
-- Project back does not bring one back (FEA-061). From now on every change that ends a participation
-- clears those rows in the same transaction; this removes the rows that earlier changes left behind
-- (a removed grant, an unlinked Team, a Team the person left). The channel's access checks already
-- ignore such rows (FEA-060), so nothing that worked until now stops working: only the rows go, and
-- with them a Manager role that would otherwise return with the participation. Messages and the
-- per-person settings (read position, mutes, favorites, later items, notification settings) stay.
DELETE FROM penfii.channel_members cm
USING penfii.channels ch
WHERE ch.id = cm.channel_id
  AND ch.project_id IS NOT NULL
  AND NOT penfii.in_share_scope(ch.project_id, ch.team_id, cm.user_id);

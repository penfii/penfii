-- FEA-149, follow-up to 0004. Removing a person from a Workspace now also revokes
--   * every participant row they were given on a Doc page, a MANAGER row too, even when no other
--     manager would remain (0004 kept the last manager row of such a page; the page stays and a
--     Workspace owner or admin names a manager for it),
--   * their rows in `workspace_security_auditors` (read access to the audit log) for that Workspace,
--   * their rows in `channel_content_auditors` (read access to a private channel's content) for the
--     channels of that Workspace.
-- This file does the same for people who were removed before, whose rows are still there. It only
-- touches a person whose membership of that Workspace is INACTIVE right now (a person who was taken
-- back keeps what they hold), and it is scoped by the Workspace of each row, so other Workspaces are
-- untouched. As in 0004 it leaves alone the participant row on a page the person created
-- (author-based access), direct messages, and everything the person wrote or did, and it never
-- deletes audit entries. Applying it again changes nothing: the rows are gone, and an audit entry
-- (`member.access_cleaned`, `via: migration 0007`) is only added for rows this run deleted.

WITH docs_gone AS (
    DELETE FROM penfii.doc_page_participants pp
    USING penfii.doc_pages p, penfii.workspace_members wm
    WHERE pp.page_id = p.id
      AND wm.workspace_id = p.workspace_id AND wm.user_id = pp.user_id AND wm.state = 'INACTIVE'
      AND p.created_by <> pp.user_id
    RETURNING p.workspace_id, pp.user_id, 'docShares'::text AS kind
), security_gone AS (
    DELETE FROM penfii.workspace_security_auditors a
    USING penfii.workspace_members wm
    WHERE wm.workspace_id = a.workspace_id AND wm.user_id = a.user_id AND wm.state = 'INACTIVE'
    RETURNING a.workspace_id, a.user_id, 'securityAuditor'::text AS kind
), channel_auditors_gone AS (
    DELETE FROM penfii.channel_content_auditors a
    USING penfii.conversations c, penfii.workspace_members wm
    WHERE a.channel_id = c.id
      AND wm.workspace_id = c.workspace_id AND wm.user_id = a.user_id AND wm.state = 'INACTIVE'
    RETURNING c.workspace_id, a.user_id, 'channelAuditors'::text AS kind
), cleaned AS (
    SELECT * FROM docs_gone
    UNION ALL SELECT * FROM security_gone
    UNION ALL SELECT * FROM channel_auditors_gone
)
INSERT INTO penfii.audit_entries
    (id, workspace_id, actor_user_id, action_code, outcome_code, target_kind, target_id, command_id, data)
SELECT gen_random_uuid(), c.workspace_id, NULL, 'member.access_cleaned', 'OK', 'USER', c.user_id, gen_random_uuid(),
       jsonb_build_object(
           'userId', c.user_id,
           'via', 'migration 0007',
           'directAdminOf', '[]'::jsonb,
           'revoked', jsonb_build_object(
               'teams', 0,
               'projects', 0,
               'groups', 0,
               'docShares', count(*) FILTER (WHERE c.kind = 'docShares'),
               'securityAuditor', count(*) FILTER (WHERE c.kind = 'securityAuditor'),
               'channelAuditors', count(*) FILTER (WHERE c.kind = 'channelAuditors')))
FROM cleaned c
GROUP BY c.workspace_id, c.user_id;

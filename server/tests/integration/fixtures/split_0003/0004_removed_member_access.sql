-- FEA-149: a person who is removed from a Workspace loses the access that was explicitly given to
-- them there, so coming back (a new invitation, or the operator adding them) starts from a new
-- membership instead of restoring the old one. From this release the removal itself does it. This
-- file does the same for the people removed by earlier releases, whose rows were left behind:
--   * Team memberships of the Workspace's Teams,
--   * direct Project grants on the Workspace's Projects,
--   * Group memberships of the Workspace's Groups,
--   * invited participants of Doc pages of the Workspace.
-- It only touches a person whose membership of that Workspace is INACTIVE right now. A person who was
-- removed and has since been taken back keeps what they hold. It never deletes records of what the
-- person wrote or did (messages, Docs, Tickets, audit entries), and it leaves alone:
--   * Doc participant rows on a page the person created (the author keeps their access to their own page),
--   * direct messages (`dm_members` is a relation, not a grant),
--   * the last manager row of a Doc page that is shared with a Project or with invited people, when no
--     other manager row would remain: deleting it would leave a page nobody can manage. It has no
--     effect while the person is not a member, and is the one explicit right that can reappear on a return.
--   * accounts' rights in any other Workspace (every statement is scoped by the Workspace of the row).
-- The audit entry also names the Projects where the person was a direct admin, which the list of
-- Projects without an admin reads to say who held the role.
-- Channel memberships were already cleared on removal and are not touched. Applying the file again
-- changes nothing: the rows it removes are gone, and it only adds an audit entry for rows it deleted.

WITH teams_gone AS (
    DELETE FROM penfii.team_members tm
    USING penfii.teams t, penfii.workspace_members wm
    WHERE tm.team_id = t.id
      AND wm.workspace_id = t.workspace_id AND wm.user_id = tm.user_id AND wm.state = 'INACTIVE'
    RETURNING t.workspace_id, tm.user_id, 'teams'::text AS kind, NULL::uuid AS admin_of
), projects_gone AS (
    DELETE FROM penfii.project_user_grants g
    USING penfii.projects p, penfii.workspace_members wm
    WHERE g.project_id = p.id
      AND wm.workspace_id = p.workspace_id AND wm.user_id = g.user_id AND wm.state = 'INACTIVE'
    RETURNING p.workspace_id, g.user_id, 'projects'::text AS kind,
              CASE WHEN g.role IN ('ADMIN', 'OWNER') THEN g.project_id END AS admin_of
), groups_gone AS (
    DELETE FROM penfii.group_members gm
    USING penfii.groups gr, penfii.workspace_members wm
    WHERE gm.group_id = gr.id
      AND wm.workspace_id = gr.workspace_id AND wm.user_id = gm.user_id AND wm.state = 'INACTIVE'
    RETURNING gr.workspace_id, gm.user_id, 'groups'::text AS kind, NULL::uuid AS admin_of
), docs_gone AS (
    DELETE FROM penfii.doc_page_participants pp
    USING penfii.doc_pages p, penfii.workspace_members wm
    WHERE pp.page_id = p.id
      AND wm.workspace_id = p.workspace_id AND wm.user_id = pp.user_id AND wm.state = 'INACTIVE'
      AND p.created_by <> pp.user_id
      AND (
            pp.access_role <> 'MANAGER'
            OR p.visibility = 'WORKSPACE'
            OR EXISTS (
                SELECT 1 FROM penfii.doc_page_participants o
                WHERE o.page_id = p.id AND o.access_role = 'MANAGER'
                  AND (o.user_id = p.created_by
                       OR NOT EXISTS (SELECT 1 FROM penfii.workspace_members w2
                                      WHERE w2.workspace_id = p.workspace_id AND w2.user_id = o.user_id
                                        AND w2.state = 'INACTIVE'))
            )
          )
    RETURNING p.workspace_id, pp.user_id, 'docShares'::text AS kind, NULL::uuid AS admin_of
), cleaned AS (
    SELECT * FROM teams_gone
    UNION ALL SELECT * FROM projects_gone
    UNION ALL SELECT * FROM groups_gone
    UNION ALL SELECT * FROM docs_gone
)
INSERT INTO penfii.audit_entries
    (id, workspace_id, actor_user_id, action_code, outcome_code, target_kind, target_id, command_id, data)
SELECT gen_random_uuid(), c.workspace_id, NULL, 'member.access_cleaned', 'OK', 'USER', c.user_id, gen_random_uuid(),
       jsonb_build_object(
           'userId', c.user_id,
           'via', 'migration 0004',
           'directAdminOf', COALESCE(jsonb_agg(c.admin_of) FILTER (WHERE c.admin_of IS NOT NULL), '[]'::jsonb),
           'revoked', jsonb_build_object(
               'teams', count(*) FILTER (WHERE c.kind = 'teams'),
               'projects', count(*) FILTER (WHERE c.kind = 'projects'),
               'groups', count(*) FILTER (WHERE c.kind = 'groups'),
               'docShares', count(*) FILTER (WHERE c.kind = 'docShares')))
FROM cleaned c
GROUP BY c.workspace_id, c.user_id;

-- Per-person rows of the access a removal must take away, and totals; run before and after the upgrade.
SELECT u.email_key AS who,
 (SELECT string_agg(w.name||':'||wm.state, ',' ORDER BY w.name) FROM penfii.workspace_members wm JOIN penfii.workspaces w ON w.id=wm.workspace_id WHERE wm.user_id=u.id) AS membership,
 (SELECT count(*) FROM penfii.team_members WHERE user_id=u.id) AS team_rows,
 (SELECT count(*) FROM penfii.project_user_grants WHERE user_id=u.id) AS grant_rows,
 (SELECT count(*) FROM penfii.group_members WHERE user_id=u.id) AS group_rows,
 (SELECT count(*) FROM penfii.doc_page_participants WHERE user_id=u.id) AS doc_rows,
 (SELECT count(*) FROM penfii.doc_page_participants WHERE user_id=u.id AND access_role='MANAGER') AS mgr_rows,
 (SELECT count(*) FROM penfii.workspace_security_auditors WHERE user_id=u.id) AS ws_aud,
 (SELECT count(*) FROM penfii.channel_content_auditors WHERE user_id=u.id) AS ch_aud,
 (SELECT count(*) FROM penfii.dm_members WHERE user_id=u.id) AS dm_rows,
 (SELECT count(*) FROM penfii.channel_members WHERE user_id=u.id) AS chan_rows
FROM penfii.users u ORDER BY u.email_key;
SELECT 'audit_entries='||count(*)||' messages='||(SELECT count(*) FROM penfii.messages)||' docs='||(SELECT count(*) FROM penfii.doc_pages) FROM penfii.audit_entries;

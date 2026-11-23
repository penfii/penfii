-- What a removal in the previous release left behind, for the FEA-149 upgrade proof (see setup.sh).
-- Run by setup.sh with psql after the people exist; prints the ids it made.
SELECT id AS owner FROM penfii.users WHERE email_key='owner@up.test' \gset
SELECT id AS active FROM penfii.users WHERE email_key='active@up.test' \gset
SELECT id AS gone FROM penfii.users WHERE email_key='gone@up.test' \gset
SELECT id AS gone2 FROM penfii.users WHERE email_key='gone2@up.test' \gset
SELECT workspace_id AS w1 FROM penfii.workspace_members WHERE user_id=:'owner' \gset
SELECT gen_random_uuid() AS w2 \gset
SELECT gen_random_uuid() AS t1 \gset
SELECT gen_random_uuid() AS t2 \gset
SELECT gen_random_uuid() AS p1 \gset
SELECT gen_random_uuid() AS p2 \gset
SELECT gen_random_uuid() AS p3 \gset
SELECT gen_random_uuid() AS p4 \gset
SELECT gen_random_uuid() AS g1 \gset
SELECT gen_random_uuid() AS g2 \gset
SELECT gen_random_uuid() AS d1 \gset
SELECT gen_random_uuid() AS d2 \gset
SELECT gen_random_uuid() AS d3 \gset
SELECT gen_random_uuid() AS d4 \gset
SELECT gen_random_uuid() AS d6 \gset
SELECT gen_random_uuid() AS c1 \gset
SELECT gen_random_uuid() AS dm \gset

INSERT INTO penfii.workspaces (id,name,time_zone) VALUES (:'w2','W2','UTC');
INSERT INTO penfii.workspace_auth_state (workspace_id) VALUES (:'w2');
INSERT INTO penfii.workspace_graph_state (workspace_id) VALUES (:'w2');
INSERT INTO penfii.workspace_members (workspace_id,user_id,role,state) VALUES
 (:'w2',:'owner','OWNER','ACTIVE'),(:'w2',:'gone','MEMBER','ACTIVE'),(:'w2',:'gone2','MEMBER','ACTIVE');
-- v0.3.1 removal: only the membership flips.
UPDATE penfii.workspace_members SET state='INACTIVE', deactivated_at=now()
 WHERE workspace_id=:'w1' AND user_id IN (:'gone',:'gone2');
INSERT INTO penfii.audit_entries (id,workspace_id,actor_user_id,action_code,outcome_code,target_kind,target_id,command_id,data) VALUES
 (gen_random_uuid(),:'w1',:'owner','member.removed','OK','USER',:'gone',gen_random_uuid(),jsonb_build_object('userId',:'gone'::text,'role','MEMBER')),
 (gen_random_uuid(),:'w1',:'owner','member.removed','OK','USER',:'gone2',gen_random_uuid(),jsonb_build_object('userId',:'gone2'::text,'role','MEMBER'));

INSERT INTO penfii.teams (id,workspace_id,name) VALUES (:'t1',:'w1','T1'),(:'t2',:'w2','T2');
INSERT INTO penfii.team_members (team_id,user_id,role) VALUES
 (:'t1',:'gone','MEMBER'),(:'t1',:'gone2','MEMBER'),(:'t1',:'active','MEMBER'),(:'t2',:'gone','MEMBER'),(:'t2',:'gone2','MEMBER');
INSERT INTO penfii.groups (id,workspace_id,name) VALUES (:'g1',:'w1','G1'),(:'g2',:'w2','G2');
INSERT INTO penfii.group_members (group_id,user_id) VALUES (:'g1',:'gone'),(:'g1',:'active'),(:'g2',:'gone');

INSERT INTO penfii.projects (id,workspace_id,name,ticket_prefix) VALUES
 (:'p1',:'w1','ByGrant','PA'),(:'p2',:'w1','ByTeam','PB'),(:'p3',:'w1','ByGroup','PC'),(:'p4',:'w2','OtherWs','PD');
INSERT INTO penfii.workflows (project_id) SELECT unnest(ARRAY[:'p1',:'p2',:'p3',:'p4']::uuid[]);
INSERT INTO penfii.workflow_statuses (id,project_id,name,category,position,is_initial)
 SELECT gen_random_uuid(), p, 'Todo','NOT_STARTED',1,true FROM unnest(ARRAY[:'p1',:'p2',:'p3',:'p4']::uuid[]) p;
INSERT INTO penfii.project_user_grants (project_id,user_id,role) VALUES
 (:'p1',:'owner','ADMIN'),(:'p1',:'active','EDITOR'),(:'p1',:'gone','ADMIN'),(:'p1',:'gone2','EDITOR'),
 (:'p2',:'owner','ADMIN'),(:'p3',:'owner','ADMIN'),
 (:'p4',:'owner','ADMIN'),(:'p4',:'gone','EDITOR'),(:'p4',:'gone2','VIEWER');
INSERT INTO penfii.project_teams (project_id,team_id,access_role) VALUES (:'p2',:'t1','EDITOR');
INSERT INTO penfii.project_group_grants (project_id,group_id,role) VALUES (:'p3',:'g1','VIEWER');

INSERT INTO penfii.doc_pages (id,workspace_id,title,visibility,content,content_schema_version,content_hash,created_by) VALUES
 (:'d1',:'w1','D1 last manager gone','RESTRICTED','{"schema":"penfii.doc","version":1,"blocks":[]}',1,'h',:'active'),
 (:'d2',:'w1','D2 shared with gone','RESTRICTED','{"schema":"penfii.doc","version":1,"blocks":[]}',1,'h',:'owner'),
 (:'d3',:'w1','D3 authored by gone','RESTRICTED','{"schema":"penfii.doc","version":1,"blocks":[]}',1,'h',:'gone'),
 (:'d4',:'w1','D4 last manager gone2','RESTRICTED','{"schema":"penfii.doc","version":1,"blocks":[]}',1,'h',:'active'),
 (:'d6',:'w2','D6 other workspace','RESTRICTED','{"schema":"penfii.doc","version":1,"blocks":[]}',1,'h',:'owner');
INSERT INTO penfii.doc_page_participants (page_id,user_id,access_role) VALUES
 (:'d1',:'gone','MANAGER'),(:'d1',:'active','EDITOR'),
 (:'d2',:'owner','MANAGER'),(:'d2',:'gone','EDITOR'),(:'d2',:'active','VIEWER'),
 (:'d3',:'gone','MANAGER'),(:'d3',:'active','EDITOR'),
 (:'d4',:'gone2','MANAGER'),(:'d4',:'active','EDITOR'),
 (:'d6',:'owner','MANAGER'),(:'d6',:'gone','EDITOR');

INSERT INTO penfii.conversations (id,workspace_id,kind,last_message_seq) VALUES (:'c1',:'w1','CHANNEL',1),(:'dm',:'w1','DM',2);
INSERT INTO penfii.channels (id,name,visibility) VALUES (:'c1','secret','PRIVATE');
INSERT INTO penfii.channel_members (channel_id,user_id,role) VALUES (:'c1',:'owner','MANAGER'),(:'c1',:'gone','MEMBER');
INSERT INTO penfii.dm_members (conversation_id,user_id) VALUES (:'dm',:'gone'),(:'dm',:'active');
INSERT INTO penfii.messages (id,conversation_id,stream_seq,author_id,body_schema_version,body) VALUES
 (gen_random_uuid(),:'c1',1,:'owner',1,'{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"private-channel-secret"}]}]}'),
 (gen_random_uuid(),:'dm',1,:'gone',1,'{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"gone wrote this in the dm"}]}]}'),
 (gen_random_uuid(),:'dm',2,:'active',1,'{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"active replied in the dm"}]}]}');

INSERT INTO penfii.workspace_security_auditors (workspace_id,user_id) VALUES
 (:'w1',:'gone'),(:'w1',:'gone2'),(:'w1',:'active'),(:'w2',:'gone'),(:'w2',:'gone2');
INSERT INTO penfii.channel_content_auditors (channel_id,user_id) VALUES (:'c1',:'gone'),(:'c1',:'active');

SELECT 'W1='||:'w1'||' W2='||:'w2'||' P1='||:'p1'||' P2='||:'p2'||' P3='||:'p3'||' P4='||:'p4'||' T1='||:'t1'||' D1='||:'d1'||' D2='||:'d2'||' D3='||:'d3'||' D4='||:'d4'||' D6='||:'d6'||' C1='||:'c1'||' DM='||:'dm';

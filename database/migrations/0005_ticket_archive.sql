-- Ticket archive. `archived_at` already exists and is independent of `trashed_at` and `purged_at`;
-- this records who archived and adds the Sprint-completion archive schedule.

ALTER TABLE penfii.tickets
    ADD COLUMN archived_by uuid;

ALTER TABLE ONLY penfii.tickets
    ADD CONSTRAINT tickets_archived_by_fkey FOREIGN KEY (archived_by) REFERENCES penfii.users(id);

-- One row per Sprint at most: when the Tickets that ended DONE or CANCELED in that Sprint are archived.
-- Immediate archives run inside the completion command and write no row. A due SCHEDULED row is
-- picked up by the same polling loop pattern as reminders (SKIP LOCKED, one transaction per row), so
-- a server that was down at the due time archives afterwards instead of dropping the schedule.
CREATE TABLE penfii.sprint_archive_schedules (
    sprint_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    project_id uuid NOT NULL,
    scheduled_by uuid NOT NULL,
    due_at timestamp with time zone NOT NULL,
    time_zone text NOT NULL,
    state text DEFAULT 'SCHEDULED'::text NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    archived_count integer,
    skipped_count integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    finished_at timestamp with time zone,
    CONSTRAINT sprint_archive_schedules_pkey PRIMARY KEY (sprint_id),
    CONSTRAINT sprint_archive_schedules_state_check CHECK ((state = ANY (ARRAY['SCHEDULED'::text, 'DONE'::text, 'CANCELED'::text]))),
    CONSTRAINT sprint_archive_schedules_sprint_fkey FOREIGN KEY (sprint_id) REFERENCES penfii.sprints(id),
    CONSTRAINT sprint_archive_schedules_project_fkey FOREIGN KEY (project_id) REFERENCES penfii.projects(id),
    CONSTRAINT sprint_archive_schedules_user_fkey FOREIGN KEY (scheduled_by) REFERENCES penfii.users(id)
);

CREATE INDEX sprint_archive_schedules_due_idx ON penfii.sprint_archive_schedules USING btree (due_at, sprint_id) WHERE (state = 'SCHEDULED'::text);

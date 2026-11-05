//! Saved Views: their definition, validation and commands.

use penfii_contracts::ProjectView;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashSet;
use uuid::Uuid;

use super::{is_unique, version_conflict};
use crate::error::CoreError;
use crate::platform;
use crate::project_role::require_project_role;

pub(super) const LIST_MAX: usize = 50;

const GROUPS: &[&str] = &[
    "NONE",
    "STATUS",
    "CATEGORY",
    "PROJECT",
    "MILESTONE",
    "ASSIGNEE",
    "PRIORITY",
    "SPRINT",
];

const SORTS: &[&str] = &[
    "RANK", "PRIORITY", "DUE_ON", "CREATED", "UPDATED", "TITLE", "ESTIMATE",
];

const PRIORITIES: &[&str] = &["LOW", "MEDIUM", "HIGH", "URGENT"];

const KINDS: &[&str] = &["EPIC", "TASK", "SUBTASK"];

const CATEGORIES: &[&str] = &["NOT_STARTED", "IN_PROGRESS", "DONE", "CANCELED"];

const LAYOUTS: &[&str] = &["BOARD", "LIST", "TABLE"];

const TABLE_COLUMNS: &[&str] = &[
    "KEY",
    "TITLE",
    "STATE",
    "ASSIGNEE",
    "PRIORITY",
    "LABELS",
    "PROJECT",
    "CHANNEL",
    "SPRINT",
    "MILESTONE",
    "PARENT",
    "DUE_ON",
    "ESTIMATE",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewFilter {
    #[serde(default)]
    pub label_ids: Vec<Uuid>,
    #[serde(default)]
    pub assignee_ids: Vec<Uuid>,
    #[serde(default)]
    pub unassigned: bool,
    #[serde(default)]
    pub priorities: Vec<String>,
    #[serde(default)]
    pub type_ids: Vec<Uuid>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub sprint_ids: Vec<Uuid>,
    #[serde(default)]
    pub no_sprint: bool,
    #[serde(default)]
    pub milestone_ids: Vec<Uuid>,
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub text: Option<String>,
    /// State categories; the same words mean the same thing in every Project.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Channels of the Project (Project scope only).
    #[serde(default)]
    pub channel_ids: Vec<Uuid>,
    /// Projects of a Team or Workspace scope.
    #[serde(default)]
    pub project_ids: Vec<Uuid>,
    /// Estimate: has one, has none, or compares (inside one Project only: units differ between
    /// Projects).
    #[serde(default)]
    pub estimate: Option<EstimateFilter>,
}

/// `op` is `SET` (has an estimate), `UNSET`, `GTE` or `LTE` (with a decimal `value`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EstimateFilter {
    pub op: String,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewSort {
    pub field: String,
    pub dir: String,
}

impl Default for ViewSort {
    fn default() -> Self {
        Self {
            field: "RANK".into(),
            dir: "ASC".into(),
        }
    }
}

/// How a View presents the Tickets it selects. Display never changes which Tickets are shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewDisplay {
    /// Show Subtasks as Tickets of their own.
    #[serde(default = "yes")]
    pub subtasks: bool,
    /// Tighter rows and cards.
    #[serde(default)]
    pub compact: bool,
    /// Properties a Table shows; empty means the default set.
    #[serde(default)]
    pub columns: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Default for ViewDisplay {
    fn default() -> Self {
        Self {
            subtasks: true,
            compact: false,
            columns: Vec::new(),
        }
    }
}

/// How a View shows Tickets: which, grouped how, in what order, presented how (schema version 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewDefinition {
    #[serde(default)]
    pub filter: ViewFilter,
    #[serde(default = "default_group")]
    pub group: String,
    #[serde(default)]
    pub sort: ViewSort,
    #[serde(default)]
    pub display: ViewDisplay,
}

fn default_group() -> String {
    "STATUS".into()
}

impl Default for ViewDefinition {
    fn default() -> Self {
        Self {
            filter: ViewFilter::default(),
            group: default_group(),
            sort: ViewSort::default(),
            display: ViewDisplay::default(),
        }
    }
}

/// Where a saved View lives. The place a person is in decides it; there is no scope picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTarget {
    Project(Uuid),
    Team { workspace_id: Uuid, team_id: Uuid },
    Workspace(Uuid),
}

impl ViewTarget {
    fn kind(self) -> &'static str {
        match self {
            Self::Project(_) => "PROJECT",
            Self::Team { .. } => "TEAM",
            Self::Workspace(_) => "WORKSPACE",
        }
    }
}

async fn count_in(
    conn: &mut PgConnection,
    sql: &str,
    ids: &[Uuid],
    scope: Uuid,
) -> Result<bool, CoreError> {
    if ids.is_empty() {
        return Ok(true);
    }
    let unique: HashSet<&Uuid> = ids.iter().collect();
    let found: i64 = sqlx::query_scalar(sql)
        .bind(ids)
        .bind(scope)
        .fetch_one(&mut *conn)
        .await?;
    Ok(found as usize == unique.len())
}

/// Checks the shape and the references of a View definition against its target and returns the
/// normal form. Ids that only mean something inside one Project (labels, types, Sprints,
/// Milestones, channels) are refused in a Team or Workspace scope.
async fn validate_definition(
    conn: &mut PgConnection,
    workspace: Uuid,
    target: ViewTarget,
    layout: &str,
    raw: Value,
) -> Result<Value, CoreError> {
    if !LAYOUTS.contains(&layout) {
        return Err(CoreError::validation("invalid layout"));
    }
    if layout == "BOARD" && !matches!(target, ViewTarget::Project(_)) {
        return Err(CoreError::validation(
            "a Board shows the States of one Project; use List or Table here",
        ));
    }
    let mut definition: ViewDefinition = serde_json::from_value(raw)
        .map_err(|e| CoreError::validation(format!("invalid view definition: {e}")))?;
    let filter = &mut definition.filter;
    for (label, len) in [
        ("labelIds", filter.label_ids.len()),
        ("assigneeIds", filter.assignee_ids.len()),
        ("priorities", filter.priorities.len()),
        ("typeIds", filter.type_ids.len()),
        ("kinds", filter.kinds.len()),
        ("sprintIds", filter.sprint_ids.len()),
        ("milestoneIds", filter.milestone_ids.len()),
        ("categories", filter.categories.len()),
        ("channelIds", filter.channel_ids.len()),
        ("projectIds", filter.project_ids.len()),
    ] {
        if len > LIST_MAX {
            return Err(CoreError::validation(format!(
                "{label} accepts at most {LIST_MAX} values"
            )));
        }
    }
    if filter
        .priorities
        .iter()
        .any(|v| !PRIORITIES.contains(&v.as_str()))
    {
        return Err(CoreError::validation("invalid priority in the filter"));
    }
    if filter.kinds.iter().any(|v| !KINDS.contains(&v.as_str())) {
        return Err(CoreError::validation("invalid ticket kind in the filter"));
    }
    if filter
        .categories
        .iter()
        .any(|v| !CATEGORIES.contains(&v.as_str()))
    {
        return Err(CoreError::validation("invalid category in the filter"));
    }
    if let Some(estimate) = filter.estimate.as_mut() {
        match (estimate.op.as_str(), estimate.value.as_deref()) {
            ("SET" | "UNSET", None) => {}
            ("GTE" | "LTE", Some(value)) => {
                if !matches!(target, ViewTarget::Project(_)) {
                    return Err(CoreError::validation(
                        "compare estimates inside one Project; units differ between Projects",
                    ));
                }
                estimate.value = Some(crate::plan_fields::format_hundredths(
                    crate::plan_fields::parse_hundredths(value)?,
                ));
            }
            _ => {
                return Err(CoreError::validation(
                    "the estimate filter is SET or UNSET without a value, or GTE or LTE with one",
                ));
            }
        }
    }
    filter.text = filter
        .text
        .take()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    if filter
        .text
        .as_ref()
        .is_some_and(|t| t.chars().count() > 100)
    {
        return Err(CoreError::validation(
            "filter text is limited to 100 characters",
        ));
    }
    if !GROUPS.contains(&definition.group.as_str()) {
        return Err(CoreError::validation("invalid group"));
    }
    if !SORTS.contains(&definition.sort.field.as_str())
        || !matches!(definition.sort.dir.as_str(), "ASC" | "DESC")
    {
        return Err(CoreError::validation("invalid sort"));
    }
    if definition
        .display
        .columns
        .iter()
        .any(|c| !TABLE_COLUMNS.contains(&c.as_str()))
        || definition.display.columns.len() > TABLE_COLUMNS.len()
    {
        return Err(CoreError::validation("invalid table column"));
    }
    let filter = &definition.filter;
    match target {
        ViewTarget::Project(project) => {
            if !filter.project_ids.is_empty() {
                return Err(CoreError::validation(
                    "a Project View cannot filter by Project",
                ));
            }
            if definition.group == "PROJECT" {
                return Err(CoreError::validation(
                    "a Project View cannot group by Project",
                ));
            }
            let ok = count_in(conn, "SELECT count(*) FROM penfii.labels WHERE id=ANY($1) AND project_id=$2 AND retired_at IS NULL", &filter.label_ids, project).await?
                && count_in(conn, "SELECT count(*) FROM penfii.ticket_types WHERE id=ANY($1) AND project_id=$2 AND retired_at IS NULL", &filter.type_ids, project).await?
                && count_in(conn, "SELECT count(*) FROM penfii.sprints WHERE id=ANY($1) AND project_id=$2", &filter.sprint_ids, project).await?
                && count_in(conn, "SELECT count(*) FROM penfii.milestones WHERE id=ANY($1) AND project_id=$2", &filter.milestone_ids, project).await?
                && count_in(conn, "SELECT count(*) FROM penfii.channels WHERE id=ANY($1) AND project_id=$2", &filter.channel_ids, project).await?
                && count_in(conn, "SELECT count(*) FROM penfii.workspace_members WHERE user_id=ANY($1) AND workspace_id=$2 AND state='ACTIVE'", &filter.assignee_ids, workspace).await?;
            if !ok {
                return Err(CoreError::validation(
                    "the filter refers to a label, type, member, channel, Sprint or Milestone outside this project",
                ));
            }
        }
        ViewTarget::Team { .. } | ViewTarget::Workspace(_) => {
            if !(filter.label_ids.is_empty()
                && filter.type_ids.is_empty()
                && filter.sprint_ids.is_empty()
                && !filter.no_sprint
                && filter.milestone_ids.is_empty()
                && filter.channel_ids.is_empty())
            {
                return Err(CoreError::validation(
                    "labels, types, Sprints, Milestones and channels belong to one Project; filter inside a Project",
                ));
            }
            if matches!(definition.group.as_str(), "STATUS") {
                return Err(CoreError::validation(
                    "group by category here; States belong to one Project",
                ));
            }
            let ok = count_in(conn, "SELECT count(*) FROM penfii.projects WHERE id=ANY($1) AND workspace_id=$2", &filter.project_ids, workspace).await?
                && count_in(conn, "SELECT count(*) FROM penfii.workspace_members WHERE user_id=ANY($1) AND workspace_id=$2 AND state='ACTIVE'", &filter.assignee_ids, workspace).await?;
            if !ok {
                return Err(CoreError::validation(
                    "the filter refers to a Project or member outside this workspace",
                ));
            }
        }
    }
    serde_json::to_value(&definition).map_err(|e| CoreError::Internal(e.to_string()))
}

fn view_name(name: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(CoreError::validation("view name must be 1..60 characters"));
    }
    Ok(name.to_owned())
}

fn view_from(
    row: &sqlx::postgres::PgRow,
    actor_id: Uuid,
    admin: bool,
) -> Result<ProjectView, CoreError> {
    let scope: String = row.try_get("scope")?;
    let owner_id: Uuid = row.try_get("owner_id")?;
    let can_edit = if scope == "PERSONAL" {
        owner_id == actor_id
    } else {
        admin
    };
    Ok(ProjectView {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        target_kind: row.try_get("target_kind")?,
        team_id: row.try_get("team_id")?,
        name: row.try_get("name")?,
        layout: row.try_get("layout")?,
        scope,
        owner_id,
        definition: row.try_get("definition")?,
        version: row.try_get("version")?,
        can_edit,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const VIEW_COLUMNS: &str = "id,project_id,target_kind,team_id,owner_id,scope,name,layout,definition,version,created_at,updated_at";

/// Resolves a target to its workspace and the actor's Project role (admin flag), checking that the
/// actor may see it.
async fn view_target_access(
    conn: &mut PgConnection,
    actor_id: Uuid,
    target: ViewTarget,
) -> Result<(Uuid, bool), CoreError> {
    match target {
        ViewTarget::Project(project) => {
            let (workspace, role) = require_project_role(conn, project, actor_id, false).await?;
            Ok((workspace, crate::project_role::can_manage(&role)))
        }
        ViewTarget::Team {
            workspace_id,
            team_id,
        } => {
            crate::access::require_member(conn, workspace_id, actor_id).await?;
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM penfii.teams WHERE id=$1 AND workspace_id=$2)",
            )
            .bind(team_id)
            .bind(workspace_id)
            .fetch_one(&mut *conn)
            .await?;
            if !exists {
                return Err(CoreError::NotFound);
            }
            Ok((workspace_id, false))
        }
        ViewTarget::Workspace(workspace_id) => {
            crate::access::require_member(conn, workspace_id, actor_id).await?;
            Ok((workspace_id, false))
        }
    }
}

pub async fn list_views(
    pool: &PgPool,
    actor_id: Uuid,
    target: ViewTarget,
) -> Result<Vec<ProjectView>, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, admin) = view_target_access(&mut tx, actor_id, target).await?;
    let rows = match target {
        ViewTarget::Project(project) => {
            sqlx::query(&format!(
                "SELECT {VIEW_COLUMNS} FROM penfii.project_views
                 WHERE project_id=$1 AND deleted_at IS NULL AND (scope='PROJECT' OR owner_id=$2)
                 ORDER BY scope DESC, created_at, id"
            ))
            .bind(project)
            .bind(actor_id)
            .fetch_all(&mut *tx)
            .await?
        }
        ViewTarget::Team { team_id, .. } => {
            sqlx::query(&format!(
                "SELECT {VIEW_COLUMNS} FROM penfii.project_views
                 WHERE workspace_id=$1 AND target_kind='TEAM' AND team_id=$3 AND owner_id=$2 AND deleted_at IS NULL
                 ORDER BY created_at, id"
            ))
            .bind(workspace)
            .bind(actor_id)
            .bind(team_id)
            .fetch_all(&mut *tx)
            .await?
        }
        ViewTarget::Workspace(_) => {
            sqlx::query(&format!(
                "SELECT {VIEW_COLUMNS} FROM penfii.project_views
                 WHERE workspace_id=$1 AND target_kind='WORKSPACE' AND owner_id=$2 AND deleted_at IS NULL
                 ORDER BY created_at, id"
            ))
            .bind(workspace)
            .bind(actor_id)
            .fetch_all(&mut *tx)
            .await?
        }
    };
    tx.commit().await?;
    rows.iter()
        .map(|row| view_from(row, actor_id, admin))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub async fn create_view(
    pool: &PgPool,
    actor_id: Uuid,
    target: ViewTarget,
    name: &str,
    scope: &str,
    layout: &str,
    definition: Value,
    idem: &str,
) -> Result<ProjectView, CoreError> {
    let name = view_name(name)?;
    if !matches!(scope, "PERSONAL" | "PROJECT") {
        return Err(CoreError::validation("scope must be PERSONAL or PROJECT"));
    }
    if scope == "PROJECT" && !matches!(target, ViewTarget::Project(_)) {
        return Err(CoreError::validation(
            "only a Project View can be shared with the Project",
        ));
    }
    let mut tx = pool.begin().await?;
    let (workspace, admin) = view_target_access(&mut tx, actor_id, target).await?;
    if scope == "PROJECT" && !admin {
        return Err(CoreError::forbidden(
            "only a project admin can create a shared view",
        ));
    }
    let body = json!({"target":target.kind(),"targetId":match target {
        ViewTarget::Project(p) => p,
        ViewTarget::Team { team_id, .. } => team_id,
        ViewTarget::Workspace(w) => w,
    },"name":name,"scope":scope,"layout":layout,"definition":definition});
    if let Some(done) =
        platform::replay::<ProjectView>(&mut tx, workspace, actor_id, idem, "view.create", &body)
            .await?
    {
        tx.commit().await?;
        return Ok(done);
    }
    let definition = validate_definition(&mut tx, workspace, target, layout, definition).await?;
    let id = Uuid::new_v4();
    let (project, team) = match target {
        ViewTarget::Project(p) => (Some(p), None),
        ViewTarget::Team { team_id, .. } => (None, Some(team_id)),
        ViewTarget::Workspace(_) => (None, None),
    };
    let inserted = sqlx::query(
        "INSERT INTO penfii.project_views(id,workspace_id,project_id,target_kind,team_id,owner_id,scope,name,layout,definition_schema_version,definition)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,2,$10)",
    )
    .bind(id)
    .bind(workspace)
    .bind(project)
    .bind(target.kind())
    .bind(team)
    .bind(actor_id)
    .bind(scope)
    .bind(&name)
    .bind(layout)
    .bind(&definition)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(error) if is_unique(&error) => {
            return Err(CoreError::conflict(
                "view_name_taken",
                "a view with this name already exists",
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let row = sqlx::query(&format!(
        "SELECT {VIEW_COLUMNS} FROM penfii.project_views WHERE id=$1"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let view = view_from(&row, actor_id, admin)?;
    crate::events::emit_simple(
        &mut tx,
        workspace,
        "VIEW",
        id,
        "view.created",
        Some(actor_id),
        json!({}),
    )
    .await?;
    platform::finish(&mut tx, workspace, actor_id, idem, 201, &view).await?;
    tx.commit().await?;
    Ok(view)
}

async fn editable_view(
    tx: &mut PgConnection,
    actor_id: Uuid,
    view_id: Uuid,
) -> Result<(sqlx::postgres::PgRow, ViewTarget, bool), CoreError> {
    let row = sqlx::query(&format!(
        "SELECT {VIEW_COLUMNS},workspace_id FROM penfii.project_views WHERE id=$1 AND deleted_at IS NULL FOR UPDATE"
    ))
    .bind(view_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let workspace_id: Uuid = row.try_get("workspace_id")?;
    let project_id: Option<Uuid> = row.try_get("project_id")?;
    let team_id: Option<Uuid> = row.try_get("team_id")?;
    let kind: String = row.try_get("target_kind")?;
    let target = match (kind.as_str(), project_id, team_id) {
        ("PROJECT", Some(project), _) => ViewTarget::Project(project),
        ("TEAM", _, Some(team_id)) => ViewTarget::Team {
            workspace_id,
            team_id,
        },
        _ => ViewTarget::Workspace(workspace_id),
    };
    let (_, admin) = view_target_access(tx, actor_id, target).await?;
    let scope: String = row.try_get("scope")?;
    let owner: Uuid = row.try_get("owner_id")?;
    if scope == "PERSONAL" && owner != actor_id {
        return Err(CoreError::NotFound);
    }
    let allowed = if scope == "PERSONAL" {
        owner == actor_id
    } else {
        admin
    };
    if !allowed {
        return Err(CoreError::forbidden("you cannot change this view"));
    }
    Ok((row, target, admin))
}

pub async fn update_view(
    pool: &PgPool,
    actor_id: Uuid,
    view_id: Uuid,
    expected_version: i64,
    name: Option<String>,
    layout: Option<String>,
    definition: Option<Value>,
) -> Result<ProjectView, CoreError> {
    let mut tx = pool.begin().await?;
    let (row, target, admin) = editable_view(&mut tx, actor_id, view_id).await?;
    let workspace: Uuid = row.try_get("workspace_id")?;
    let current: i64 = row.try_get("version")?;
    if current != expected_version {
        return Err(version_conflict(current));
    }
    let name = match name {
        Some(name) => view_name(&name)?,
        None => row.try_get("name")?,
    };
    let layout: String = match layout {
        Some(layout) => layout,
        None => row.try_get("layout")?,
    };
    let definition = match definition {
        Some(raw) => raw,
        None => row.try_get("definition")?,
    };
    let definition = validate_definition(&mut tx, workspace, target, &layout, definition).await?;
    let updated = sqlx::query(
        "UPDATE penfii.project_views SET name=$2,layout=$3,definition=$4,definition_schema_version=2,version=version+1,updated_at=now() WHERE id=$1",
    )
    .bind(view_id)
    .bind(&name)
    .bind(&layout)
    .bind(&definition)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(_) => {}
        Err(error) if is_unique(&error) => {
            return Err(CoreError::conflict(
                "view_name_taken",
                "a view with this name already exists",
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let row = sqlx::query(&format!(
        "SELECT {VIEW_COLUMNS} FROM penfii.project_views WHERE id=$1"
    ))
    .bind(view_id)
    .fetch_one(&mut *tx)
    .await?;
    let view = view_from(&row, actor_id, admin)?;
    crate::events::emit_simple(
        &mut tx,
        workspace,
        "VIEW",
        view_id,
        "view.updated",
        Some(actor_id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(view)
}

pub async fn delete_view(
    pool: &PgPool,
    actor_id: Uuid,
    view_id: Uuid,
    expected_version: i64,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let (row, _, _) = editable_view(&mut tx, actor_id, view_id).await?;
    let current: i64 = row.try_get("version")?;
    if current != expected_version {
        return Err(version_conflict(current));
    }
    sqlx::query("UPDATE penfii.project_views SET deleted_at=now(),version=version+1 WHERE id=$1")
        .bind(view_id)
        .execute(&mut *tx)
        .await?;
    let workspace: Uuid = row.try_get("workspace_id")?;
    crate::events::emit_simple(
        &mut tx,
        workspace,
        "VIEW",
        view_id,
        "view.deleted",
        Some(actor_id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

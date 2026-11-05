//! Sprint reports, Velocity and the Project status summary.
//!
//! Nothing here is stored. Every figure is read, for the person asking, from the records that
//! hold the facts: the Sprint baseline (version 2), the assignment history with its end
//! snapshots, the Ticket revisions and the history of State categories. The past is never read
//! through today's Workflow, and a Ticket the person cannot see adds nothing to any sum, count or
//! warning. A Sprint that started before baselines carried estimates has no figures.
//!
//! Formulas:
//! * start plan: the baseline Tasks that were not started or in progress at the start, by their
//!   start estimate;
//! * completed: Tasks still assigned at the end, done at the end, whose final DONE began inside
//!   their last assignment and after the start, by their end estimate;
//! * plan adherence: the start estimates of the planned Tasks that were completed, over the start
//!   plan (nothing to measure when the plan is 0);
//! * remaining at a time: assigned Tasks whose category then was not done or canceled, by their
//!   estimate then.

use chrono::{DateTime, NaiveDate, Utc};
use penfii_contracts::{
    BurndownPoint, PlanAdherence, ProjectStatusSummary, ReportAmount, ReportAvailability,
    ReportTicket, ScopeChange, Sprint, SprintReport, StatusDistribution, TicketRef, VelocityEntry,
    VelocityReport,
};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::error::CoreError;
use crate::plan_fields::{format_hundredths, parse_hundredths};
use crate::planning::{SPRINT_COLUMNS, sprint_from, visible};
use crate::project_role::require_project_role;

const SCOPE: &str = "VISIBLE_TICKETS";
const VELOCITY_DEFAULT: i64 = 6;
const VELOCITY_MAX: i64 = 12;
/// Most daily points one report draws; a Sprint is weeks long, this is a guard.
const DAYS_MAX: i64 = 400;
pub const DUE_SOON_DAYS: i32 = 7;

// ---------------------------------------------------------------------------------------------
// Pure part: facts in, figures out
// ---------------------------------------------------------------------------------------------

/// An estimate in hundredths and its unit.
type Estimate = Option<(i64, String)>;

#[derive(Debug, Clone)]
pub(crate) struct Rev {
    pub at: DateTime<Utc>,
    pub status: Option<Uuid>,
    pub estimate: Estimate,
}

/// State category history per State: `(effective from, category)`, oldest first; `None` is the
/// value known when the history began and holds for all earlier time.
pub(crate) type History = HashMap<Uuid, Vec<(Option<DateTime<Utc>>, String)>>;

#[derive(Debug, Clone)]
pub(crate) struct Assign {
    pub ticket: Uuid,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub outcome: Option<String>,
    pub end_category: Option<String>,
    pub end_estimate: Estimate,
}

#[derive(Debug, Clone)]
pub(crate) struct BaseRow {
    pub category: String,
    pub estimate: Estimate,
}

pub(crate) struct Input {
    pub unit: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
    pub active: bool,
    pub tickets: HashMap<Uuid, TicketRef>,
    pub baseline: HashMap<Uuid, BaseRow>,
    pub assigns: Vec<Assign>,
    pub revs: HashMap<Uuid, Vec<Rev>>,
    pub history: History,
    /// The days of the Sprint in its zone and the instant each one closes.
    pub days: Vec<(NaiveDate, DateTime<Utc>)>,
}

pub(crate) struct Figures {
    pub start_plan: ReportAmount,
    pub completed: ReportAmount,
    pub remaining: ReportAmount,
    pub adherence: PlanAdherence,
    pub burndown: Vec<BurndownPoint>,
    pub changes: Vec<ScopeChange>,
    pub tickets: Vec<ReportTicket>,
}

#[derive(Default)]
struct Acc {
    sum: i64,
    tasks: i64,
    unestimated: i64,
    other_unit: i64,
}

impl Acc {
    fn add(&mut self, estimate: &Estimate, unit: &str) {
        self.tasks += 1;
        if unit == "NONE" {
            self.sum += 100;
            return;
        }
        match estimate {
            None => self.unestimated += 1,
            Some((value, u)) if u == unit => self.sum += value,
            Some(_) => self.other_unit += 1,
        }
    }

    fn amount(&self) -> ReportAmount {
        ReportAmount {
            value: format_hundredths(self.sum),
            tasks: self.tasks,
            unestimated: self.unestimated,
            other_unit: self.other_unit,
        }
    }
}

fn is_open(category: &str) -> bool {
    matches!(category, "NOT_STARTED" | "IN_PROGRESS")
}

fn rev_at(revs: &[Rev], at: DateTime<Utc>) -> Option<&Rev> {
    let n = revs.partition_point(|rev| rev.at <= at);
    n.checked_sub(1).map(|i| &revs[i])
}

fn category_of(history: &History, status: Uuid, at: DateTime<Utc>) -> Option<&str> {
    let rows = history.get(&status)?;
    let n = rows.partition_point(|(from, _)| from.is_none_or(|f| f <= at));
    rows.get(n.saturating_sub(1))
        .map(|(_, category)| category.as_str())
}

fn category_at<'a>(revs: &[Rev], history: &'a History, at: DateTime<Utc>) -> Option<&'a str> {
    category_of(history, rev_at(revs, at)?.status?, at)
}

fn estimate_at(revs: &[Rev], at: DateTime<Utc>) -> Estimate {
    rev_at(revs, at).and_then(|rev| rev.estimate.clone())
}

/// When the DONE the Ticket is in at `end` began: the first of the unbroken run of revisions in
/// the DONE category that ends at `end`. `None` when it is not done then.
fn done_since(revs: &[Rev], history: &History, end: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let upto = revs.partition_point(|rev| rev.at <= end);
    let mut since = None;
    for rev in revs[..upto].iter().rev() {
        let done = rev
            .status
            .and_then(|status| category_of(history, status, rev.at))
            == Some("DONE");
        if !done {
            break;
        }
        since = Some(rev.at);
    }
    since
}

/// The estimate as the Sprint's unit shows it: `(text, written in another unit)`.
fn shown(estimate: &Estimate, unit: &str) -> (Option<String>, bool) {
    match estimate {
        Some((value, u)) if unit != "NONE" && u == unit => (Some(format_hundredths(*value)), false),
        Some(_) if unit != "NONE" => (None, true),
        _ => (None, false),
    }
}

pub(crate) fn compute(input: &Input) -> Figures {
    let unit = input.unit.as_str();
    let end = input.ended_at.unwrap_or(input.now);
    let empty: Vec<Rev> = Vec::new();
    let revs_of = |ticket: &Uuid| input.revs.get(ticket).unwrap_or(&empty);

    // Start plan.
    let mut plan = Acc::default();
    for row in input.baseline.values() {
        if is_open(&row.category) {
            plan.add(&row.estimate, unit);
        }
    }

    // One row per Ticket, from its last assignment.
    let mut order: Vec<Uuid> = Vec::new();
    let mut last: HashMap<Uuid, &Assign> = HashMap::new();
    let mut first_added: HashMap<Uuid, DateTime<Utc>> = HashMap::new();
    for assign in &input.assigns {
        if !input.tickets.contains_key(&assign.ticket) {
            continue;
        }
        if !last.contains_key(&assign.ticket) {
            order.push(assign.ticket);
        }
        last.insert(assign.ticket, assign);
        if assign.from > input.started_at {
            first_added.entry(assign.ticket).or_insert(assign.from);
        }
    }

    let mut completed = Acc::default();
    let mut from_plan = Acc::default();
    let mut rows = Vec::new();
    for ticket in &order {
        let assign = last[ticket];
        let revs = revs_of(ticket);
        let base = input.baseline.get(ticket);
        let still_assigned = match (&assign.to, &assign.reason) {
            (None, _) => input.active,
            (Some(_), Some(reason)) => reason == "SPRINT_COMPLETED",
            (Some(_), None) => false,
        };
        // What the Ticket was at the end: frozen at close, read from the timeline while it runs.
        let (end_category, end_estimate) = if assign.to.is_some() {
            (
                assign.end_category.clone().or_else(|| {
                    category_at(revs, &input.history, assign.to.unwrap_or(end)).map(str::to_owned)
                }),
                assign
                    .end_estimate
                    .clone()
                    .or_else(|| estimate_at(revs, assign.to.unwrap_or(end))),
            )
        } else {
            (
                category_at(revs, &input.history, end).map(str::to_owned),
                estimate_at(revs, end),
            )
        };
        let window_start = assign.from.max(input.started_at);
        let window_end = assign.to.unwrap_or(end);
        let removed = matches!(assign.reason.as_deref(), Some("REMOVED" | "REASSIGNED"));
        let done_at_end = end_category.as_deref() == Some("DONE");
        let began = if done_at_end {
            done_since(revs, &input.history, window_end)
        } else {
            None
        };
        let counted = still_assigned
            && done_at_end
            && began.is_some_and(|b| b >= window_start && b <= window_end);
        let reopened = !done_at_end
            && revs.iter().any(|rev| {
                rev.at >= window_start
                    && rev.at <= window_end
                    && rev
                        .status
                        .and_then(|s| category_of(&input.history, s, rev.at))
                        == Some("DONE")
            });
        let reason = if removed {
            "REMOVED"
        } else if counted {
            "COUNTED"
        } else if done_at_end && still_assigned {
            "DONE_BEFORE"
        } else if end_category.as_deref() == Some("CANCELED") {
            "CANCELED"
        } else if reopened {
            "REOPENED"
        } else if assign.to.is_none() {
            "OPEN"
        } else {
            "NOT_DONE"
        };
        if counted {
            completed.add(&end_estimate, unit);
            if let Some(base) = base.filter(|b| is_open(&b.category)) {
                from_plan.add(&base.estimate, unit);
            }
        }
        let outcome = if removed {
            Some("REMOVED".to_owned())
        } else {
            assign.outcome.clone()
        };
        let (start_estimate, start_other) =
            base.map_or((None, false), |b| shown(&b.estimate, unit));
        let (end_text, end_other) = shown(&end_estimate, unit);
        rows.push(ReportTicket {
            ticket: input.tickets[ticket].clone(),
            in_baseline: base.is_some(),
            added_at: first_added.get(ticket).copied(),
            start_category: base.map(|b| b.category.clone()),
            start_estimate,
            end_category,
            end_estimate: end_text,
            other_unit: start_other || end_other,
            outcome,
            counted,
            reason: reason.to_owned(),
        });
    }

    // Remaining at a time.
    let remaining_at = |at: DateTime<Utc>| {
        let mut acc = Acc::default();
        let mut seen: HashSet<Uuid> = HashSet::new();
        for assign in &input.assigns {
            if !input.tickets.contains_key(&assign.ticket)
                || assign.from > at
                // The instant an assignment ends it still counts: the figure at the close is what was left.
                || assign.to.is_some_and(|to| at > to)
                || !seen.insert(assign.ticket)
            {
                continue;
            }
            let revs = revs_of(&assign.ticket);
            if category_at(revs, &input.history, at).is_some_and(is_open) {
                acc.add(&estimate_at(revs, at), unit);
            }
        }
        acc
    };
    let mut burndown = vec![BurndownPoint {
        at: input.started_at,
        day: None,
        remaining: remaining_at(input.started_at).amount(),
    }];
    for (day, closes) in &input.days {
        let at = (*closes).min(end);
        if at < input.started_at {
            continue;
        }
        burndown.push(BurndownPoint {
            at,
            day: Some(*day),
            remaining: remaining_at(at).amount(),
        });
    }

    // Scope changes, in time order.
    let mut changes = Vec::new();
    for assign in &input.assigns {
        let Some(ticket) = input.tickets.get(&assign.ticket) else {
            continue;
        };
        let revs = revs_of(&assign.ticket);
        if assign.from > input.started_at {
            let (text, other) = shown(&estimate_at(revs, assign.from), unit);
            changes.push(ScopeChange {
                at: assign.from,
                kind: "ADDED".into(),
                ticket: ticket.clone(),
                estimate_from: None,
                estimate_to: text,
                other_unit: other,
            });
        }
        if let (Some(to), Some("REMOVED" | "REASSIGNED")) = (assign.to, assign.reason.as_deref()) {
            let estimate = assign
                .end_estimate
                .clone()
                .or_else(|| estimate_at(revs, to));
            let (text, other) = shown(&estimate, unit);
            changes.push(ScopeChange {
                at: to,
                kind: "REMOVED".into(),
                ticket: ticket.clone(),
                estimate_from: text,
                estimate_to: None,
                other_unit: other,
            });
        }
        let (from, to) = (
            assign.from.max(input.started_at),
            assign.to.unwrap_or(end).min(end),
        );
        for (i, rev) in revs.iter().enumerate().skip(1) {
            if rev.at > from && rev.at <= to && rev.estimate != revs[i - 1].estimate {
                let (before, other_before) = shown(&revs[i - 1].estimate, unit);
                let (after, other_after) = shown(&rev.estimate, unit);
                changes.push(ScopeChange {
                    at: rev.at,
                    kind: "ESTIMATE_CHANGED".into(),
                    ticket: ticket.clone(),
                    estimate_from: before,
                    estimate_to: after,
                    other_unit: other_before || other_after,
                });
            }
        }
    }
    changes.sort_by(|a, b| {
        a.at.cmp(&b.at)
            .then_with(|| a.ticket.key.cmp(&b.ticket.key))
            .then_with(|| a.kind.cmp(&b.kind))
    });

    let percent = (plan.sum > 0).then(|| {
        let tenths = (from_plan.sum * 1000 + plan.sum / 2) / plan.sum;
        if tenths % 10 == 0 {
            (tenths / 10).to_string()
        } else {
            format!("{}.{}", tenths / 10, tenths % 10)
        }
    });
    Figures {
        start_plan: plan.amount(),
        completed: completed.amount(),
        remaining: remaining_at(end).amount(),
        adherence: PlanAdherence {
            completed_from_plan: from_plan.amount(),
            not_applicable: percent.is_none(),
            percent,
        },
        burndown,
        changes,
        tickets: rows,
    }
}

// ---------------------------------------------------------------------------------------------
// Reading the facts
// ---------------------------------------------------------------------------------------------

fn estimate_from(value: Option<String>, unit: Option<String>) -> Estimate {
    let (value, unit) = value.zip(unit)?;
    parse_hundredths(&value)
        .ok()
        .map(|hundredths| (hundredths, unit))
}

async fn report_of(
    conn: &mut PgConnection,
    actor_id: Uuid,
    sprint: Sprint,
    now: DateTime<Utc>,
) -> Result<SprintReport, CoreError> {
    let unavailable = |reason: &str, sprint: Sprint| SprintReport {
        sprint,
        availability: ReportAvailability {
            available: false,
            reason: Some(reason.to_owned()),
        },
        provisional: false,
        generated_at: now,
        scope: SCOPE.into(),
        start_plan: None,
        completed: None,
        remaining: None,
        adherence: None,
        burndown: Vec::new(),
        changes: Vec::new(),
        tickets: Vec::new(),
    };
    let baseline: Option<i16> = sqlx::query_scalar(
        "SELECT snapshot_version FROM penfii.sprint_baselines WHERE sprint_id=$1",
    )
    .bind(sprint.id)
    .fetch_optional(&mut *conn)
    .await?;
    let started_at = match (baseline, sprint.started_at) {
        (None, _) | (_, None) => return Ok(unavailable("NO_BASELINE", sprint)),
        (Some(version), _) if version < 2 => return Ok(unavailable("LEGACY_BASELINE", sprint)),
        (Some(_), Some(started)) => started,
    };

    // Assignments, and which of their Tickets the person may see.
    let rows = sqlx::query(
        "SELECT a.ticket_id, a.assigned_at, a.ended_at, a.end_reason, a.outcome, a.ended_status_category,
                a.ended_estimate_value::text AS ended_value, a.ended_estimate_unit,
                t.public_key, t.title, t.hierarchy_kind, t.channel_id
         FROM penfii.sprint_assignments a JOIN penfii.tickets t ON t.id=a.ticket_id
         WHERE a.sprint_id=$1 AND t.purged_at IS NULL ORDER BY a.assigned_at, a.id",
    )
    .bind(sprint.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut access = HashMap::new();
    let mut tickets: HashMap<Uuid, TicketRef> = HashMap::new();
    let mut assigns = Vec::new();
    for row in rows {
        let channel: Option<Uuid> = row.try_get("channel_id")?;
        if !visible(conn, &mut access, channel, actor_id).await? {
            continue;
        }
        let ticket: Uuid = row.try_get("ticket_id")?;
        tickets.entry(ticket).or_insert(TicketRef {
            id: ticket,
            key: row.try_get("public_key")?,
            title: row.try_get("title")?,
            hierarchy_kind: row.try_get("hierarchy_kind")?,
        });
        assigns.push(Assign {
            ticket,
            from: row.try_get("assigned_at")?,
            to: row.try_get("ended_at")?,
            reason: row.try_get("end_reason")?,
            outcome: row.try_get("outcome")?,
            end_category: row.try_get("ended_status_category")?,
            end_estimate: estimate_from(
                row.try_get("ended_value")?,
                row.try_get("ended_estimate_unit")?,
            ),
        });
    }
    let ids: Vec<Uuid> = tickets.keys().copied().collect();

    let mut baseline_rows = HashMap::new();
    for row in sqlx::query("SELECT ticket_id, status_category, estimate_value::text AS value, estimate_unit FROM penfii.sprint_baseline_tickets WHERE sprint_id=$1 AND ticket_id=ANY($2)")
        .bind(sprint.id)
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?
    {
        let category: Option<String> = row.try_get("status_category")?;
        if let Some(category) = category {
            baseline_rows.insert(row.try_get("ticket_id")?, BaseRow { category, estimate: estimate_from(row.try_get("value")?, row.try_get("estimate_unit")?) });
        }
    }

    let end = sprint.ended_at.unwrap_or(now);
    let mut revs: HashMap<Uuid, Vec<Rev>> = HashMap::new();
    let mut statuses: HashSet<Uuid> = HashSet::new();
    for row in sqlx::query(
        "SELECT ticket_id, recorded_at, snapshot->>'statusId' AS status,
                snapshot->>'estimateValue' AS value, snapshot->>'estimateUnit' AS unit
         FROM penfii.ticket_revisions WHERE ticket_id=ANY($1) AND recorded_at<=$2
         ORDER BY ticket_id, revision_no",
    )
    .bind(&ids)
    .bind(end)
    .fetch_all(&mut *conn)
    .await?
    {
        let status = row
            .try_get::<Option<String>, _>("status")?
            .and_then(|s| Uuid::parse_str(&s).ok());
        statuses.extend(status);
        revs.entry(row.try_get("ticket_id")?)
            .or_default()
            .push(Rev {
                at: row.try_get("recorded_at")?,
                status,
                estimate: estimate_from(row.try_get("value")?, row.try_get("unit")?),
            });
    }
    let status_ids: Vec<Uuid> = statuses.into_iter().collect();
    let mut history: History = HashMap::new();
    for row in sqlx::query(
        "SELECT status_id, category, CASE WHEN effective_from='-infinity' THEN NULL ELSE effective_from END AS from_at
         FROM penfii.workflow_status_history WHERE status_id=ANY($1) ORDER BY status_id, effective_from, id",
    )
    .bind(&status_ids)
    .fetch_all(&mut *conn)
    .await?
    {
        history.entry(row.try_get("status_id")?).or_default().push((row.try_get("from_at")?, row.try_get("category")?));
    }

    // The days of the Sprint on its own clock, each with the instant it closes.
    let days = sqlx::query(
        "SELECT g::date AS day, ((g::date + 1)::timestamp AT TIME ZONE $1) AS closes
         FROM generate_series(($2::timestamptz AT TIME ZONE $1)::date, ($3::timestamptz AT TIME ZONE $1)::date, interval '1 day') g
         LIMIT $4",
    )
    .bind(&sprint.time_zone)
    .bind(started_at)
    .bind(end)
    .bind(DAYS_MAX)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| Ok((row.try_get::<NaiveDate, _>("day")?, row.try_get::<DateTime<Utc>, _>("closes")?)))
    .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let active = sprint.state == "ACTIVE";
    let figures = compute(&Input {
        unit: sprint.estimate_unit.clone(),
        started_at,
        ended_at: sprint.ended_at,
        now,
        active,
        tickets,
        baseline: baseline_rows,
        assigns,
        revs,
        history,
        days,
    });
    Ok(SprintReport {
        sprint,
        availability: ReportAvailability {
            available: true,
            reason: None,
        },
        provisional: active,
        generated_at: now,
        scope: SCOPE.into(),
        start_plan: Some(figures.start_plan),
        completed: Some(figures.completed),
        remaining: Some(figures.remaining),
        adherence: Some(figures.adherence),
        burndown: figures.burndown,
        changes: figures.changes,
        tickets: figures.tickets,
    })
}

async fn load_sprint(conn: &mut PgConnection, sprint_id: Uuid) -> Result<Sprint, CoreError> {
    let row = sqlx::query(&format!(
        "SELECT {SPRINT_COLUMNS} FROM penfii.sprints WHERE id=$1"
    ))
    .bind(sprint_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    sprint_from(&row)
}

pub async fn sprint_report(
    pool: &PgPool,
    actor_id: Uuid,
    sprint_id: Uuid,
) -> Result<SprintReport, CoreError> {
    let mut tx = pool.begin().await?;
    let sprint = load_sprint(&mut tx, sprint_id).await?;
    require_project_role(&mut tx, sprint.project_id, actor_id, false).await?;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut *tx)
        .await?;
    let report = report_of(&mut tx, actor_id, sprint, now).await?;
    tx.commit().await?;
    Ok(report)
}

pub async fn velocity(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    limit: Option<i64>,
) -> Result<VelocityReport, CoreError> {
    let limit = crate::paging::clamp_limit(limit, VELOCITY_DEFAULT, VELOCITY_MAX);
    let mut tx = pool.begin().await?;
    require_project_role(&mut tx, project_id, actor_id, false).await?;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut *tx)
        .await?;
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM penfii.sprints WHERE project_id=$1 AND state='COMPLETED' ORDER BY ended_at DESC, id LIMIT $2")
        .bind(project_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    let mut reports = Vec::new();
    for id in ids {
        let sprint = load_sprint(&mut tx, id).await?;
        reports.push(report_of(&mut tx, actor_id, sprint, now).await?);
    }
    // Sprints are only compared with the same estimate unit and calculation policy version as the
    // newest one that has figures.
    let ids: Vec<Uuid> = reports.iter().map(|r| r.sprint.id).collect();
    let policies: HashMap<Uuid, Option<i16>> = sqlx::query_as::<_, (Uuid, Option<i16>)>(
        "SELECT sprint_id, policy_version FROM penfii.sprint_baselines WHERE sprint_id=ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    tx.commit().await?;
    let policy_of = |id: &Uuid| policies.get(id).copied().flatten();
    let newest = reports.iter().find(|r| r.availability.available);
    let unit = newest.map(|r| r.sprint.estimate_unit.clone());
    let policy_version = newest.and_then(|r| policy_of(&r.sprint.id));
    let sprints = reports
        .into_iter()
        .map(|report| {
            let (included, reason) = match (&report.availability.reason, &unit) {
                (Some(reason), _) => (false, Some(reason.clone())),
                (None, Some(unit)) if *unit != report.sprint.estimate_unit => {
                    (false, Some("DIFFERENT_UNIT".to_owned()))
                }
                (None, Some(_)) if policy_of(&report.sprint.id) != policy_version => {
                    (false, Some("DIFFERENT_POLICY".to_owned()))
                }
                _ => (true, None),
            };
            VelocityEntry {
                sprint: report.sprint,
                included,
                reason,
                completed: if included { report.completed } else { None },
            }
        })
        .collect();
    Ok(VelocityReport {
        unit,
        policy_version,
        generated_at: now,
        scope: SCOPE.into(),
        sprints,
    })
}

pub async fn status_summary(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<ProjectStatusSummary, CoreError> {
    let mut tx = pool.begin().await?;
    require_project_role(&mut tx, project_id, actor_id, false).await?;
    let (now, today): (DateTime<Utc>, NaiveDate) = sqlx::query_as(
        "SELECT now(), (now() AT TIME ZONE w.time_zone)::date FROM penfii.projects p JOIN penfii.workspaces w ON w.id=p.workspace_id WHERE p.id=$1",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.due_on, s.category FROM penfii.tickets t
         JOIN penfii.workflow_statuses s ON s.id=t.status_id
         WHERE t.project_id=$1 AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut access = HashMap::new();
    let mut seen: HashMap<Uuid, bool> = HashMap::new();
    let mut distribution = StatusDistribution {
        not_started: 0,
        in_progress: 0,
        done: 0,
        canceled: 0,
        total: 0,
    };
    let mut open: HashSet<Uuid> = HashSet::new();
    let mut due_soon = 0;
    let horizon = today + chrono::Duration::days(i64::from(DUE_SOON_DAYS));
    for row in rows {
        let channel: Option<Uuid> = row.try_get("channel_id")?;
        let id: Uuid = row.try_get("id")?;
        let can_see = visible(&mut tx, &mut access, channel, actor_id).await?;
        seen.insert(id, can_see);
        if !can_see {
            continue;
        }
        let category: String = row.try_get("category")?;
        distribution.total += 1;
        match category.as_str() {
            "NOT_STARTED" => distribution.not_started += 1,
            "IN_PROGRESS" => distribution.in_progress += 1,
            "DONE" => distribution.done += 1,
            _ => distribution.canceled += 1,
        }
        if is_open(&category) {
            open.insert(id);
            if row
                .try_get::<Option<NaiveDate>, _>("due_on")?
                .is_some_and(|due| due <= horizon)
            {
                due_soon += 1;
            }
        }
    }
    // Blocked is counted for Tasks. A blocker is a predecessor the person can see
    // that is not done and not waived.
    let edges = sqlx::query(
        "SELECT d.successor_id, pt.channel_id FROM penfii.ticket_dependencies d
         JOIN penfii.tickets st ON st.id=d.successor_id AND st.project_id=$1 AND st.hierarchy_kind='TASK'
         JOIN penfii.tickets pt ON pt.id=d.predecessor_id
              AND pt.archived_at IS NULL AND pt.trashed_at IS NULL AND pt.purged_at IS NULL
         JOIN penfii.workflow_statuses ps ON ps.id=pt.status_id
         WHERE d.kind='BLOCKS' AND d.waived_at IS NULL AND ps.category<>'DONE'",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut blocked: HashSet<Uuid> = HashSet::new();
    for edge in edges {
        let successor: Uuid = edge.try_get("successor_id")?;
        let channel: Option<Uuid> = edge.try_get("channel_id")?;
        if open.contains(&successor) && visible(&mut tx, &mut access, channel, actor_id).await? {
            blocked.insert(successor);
        }
    }
    tx.commit().await?;
    Ok(ProjectStatusSummary {
        generated_at: now,
        scope: SCOPE.into(),
        today,
        due_within_days: DUE_SOON_DAYS,
        distribution,
        blocked: blocked.len() as i64,
        due_soon,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn status(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    const TODO: u128 = 1;
    const DOING: u128 = 2;
    const DONE: u128 = 3;
    const CANCELED: u128 = 4;

    fn history() -> History {
        [
            (TODO, "NOT_STARTED"),
            (DOING, "IN_PROGRESS"),
            (DONE, "DONE"),
            (CANCELED, "CANCELED"),
        ]
        .into_iter()
        .map(|(id, category)| (status(id), vec![(None, category.to_owned())]))
        .collect()
    }

    fn rev(when: &str, state: u128, estimate: Option<i64>) -> Rev {
        Rev {
            at: at(when),
            status: Some(status(state)),
            estimate: estimate.map(|v| (v * 100, "POINTS".into())),
        }
    }

    fn reference(n: u128, key: &str) -> TicketRef {
        TicketRef {
            id: Uuid::from_u128(100 + n),
            key: key.into(),
            title: key.into(),
            hierarchy_kind: "TASK".into(),
        }
    }

    fn assign(
        n: u128,
        from: &str,
        to: Option<&str>,
        reason: Option<&str>,
        category: &str,
        estimate: Option<i64>,
    ) -> Assign {
        Assign {
            ticket: Uuid::from_u128(100 + n),
            from: at(from),
            to: to.map(at),
            reason: reason.map(str::to_owned),
            outcome: None,
            end_category: to.map(|_| category.to_owned()),
            end_estimate: to.and_then(|_| estimate.map(|v| (v * 100, "POINTS".to_owned()))),
        }
    }

    fn input(unit: &str) -> Input {
        Input {
            unit: unit.into(),
            started_at: at("2026-11-02T00:00:00Z"),
            ended_at: Some(at("2026-11-13T09:00:00Z")),
            now: at("2026-11-20T00:00:00Z"),
            active: false,
            tickets: HashMap::new(),
            baseline: HashMap::new(),
            assigns: Vec::new(),
            revs: HashMap::new(),
            history: history(),
            days: Vec::new(),
        }
    }

    fn base(category: &str, estimate: Option<i64>) -> BaseRow {
        BaseRow {
            category: category.into(),
            estimate: estimate.map(|v| (v * 100, "POINTS".into())),
        }
    }

    /// Example: A = 3 and B = 5 planned; A becomes 5, C = 2 is added; B is taken out; A and C are
    /// done. Plan 8, completed 7, adherence 3 / 8 = 37.5 %.
    fn scn019() -> Input {
        let mut i = input("POINTS");
        let (a, b, c) = (
            Uuid::from_u128(101),
            Uuid::from_u128(102),
            Uuid::from_u128(103),
        );
        i.tickets = [
            (a, reference(1, "A")),
            (b, reference(2, "B")),
            (c, reference(3, "C")),
        ]
        .into_iter()
        .collect();
        i.baseline = [
            (a, base("NOT_STARTED", Some(3))),
            (b, base("NOT_STARTED", Some(5))),
        ]
        .into_iter()
        .collect();
        i.revs = [
            (
                a,
                vec![
                    rev("2026-10-30T00:00:00Z", TODO, Some(3)),
                    rev("2026-11-04T00:00:00Z", TODO, Some(5)),
                    rev("2026-11-10T00:00:00Z", DONE, Some(5)),
                ],
            ),
            (b, vec![rev("2026-10-30T00:00:00Z", TODO, Some(5))]),
            (
                c,
                vec![
                    rev("2026-11-03T00:00:00Z", TODO, Some(2)),
                    rev("2026-11-11T12:00:00Z", DONE, Some(2)),
                ],
            ),
        ]
        .into_iter()
        .collect();
        i.assigns = vec![
            assign(
                1,
                "2026-10-31T00:00:00Z",
                Some("2026-11-13T09:00:00Z"),
                Some("SPRINT_COMPLETED"),
                "DONE",
                Some(5),
            ),
            assign(
                2,
                "2026-10-31T00:00:00Z",
                Some("2026-11-06T00:00:00Z"),
                Some("REMOVED"),
                "NOT_STARTED",
                Some(5),
            ),
            assign(
                3,
                "2026-11-03T12:00:00Z",
                Some("2026-11-13T09:00:00Z"),
                Some("SPRINT_COMPLETED"),
                "DONE",
                Some(2),
            ),
        ];
        i
    }

    #[test]
    fn scn019_plan_completed_and_adherence() {
        let figures = compute(&scn019());
        assert_eq!(figures.start_plan.value, "8");
        assert_eq!(
            figures.completed.value, "7",
            "A is counted at its end estimate 5, C at 2; B left"
        );
        assert_eq!(
            figures.adherence.completed_from_plan.value, "3",
            "A counts at its start estimate"
        );
        assert_eq!(figures.adherence.percent.as_deref(), Some("37.5"));
        assert!(!figures.adherence.not_applicable);
        let row = |key: &str| {
            figures
                .tickets
                .iter()
                .find(|t| t.ticket.key == key)
                .unwrap()
        };
        assert_eq!(
            (
                row("A").reason.as_str(),
                row("A").counted,
                row("A").start_estimate.as_deref(),
                row("A").end_estimate.as_deref()
            ),
            ("COUNTED", true, Some("3"), Some("5"))
        );
        assert_eq!(
            (row("B").reason.as_str(), row("B").counted),
            ("REMOVED", false)
        );
        assert_eq!(
            (row("C").reason.as_str(), row("C").in_baseline),
            ("COUNTED", false)
        );
        let kinds: Vec<_> = figures
            .changes
            .iter()
            .map(|c| (c.kind.as_str(), c.ticket.key.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [("ADDED", "C"), ("ESTIMATE_CHANGED", "A"), ("REMOVED", "B")]
        );
        let estimate = figures
            .changes
            .iter()
            .find(|c| c.kind == "ESTIMATE_CHANGED")
            .unwrap();
        assert_eq!(
            (
                estimate.estimate_from.as_deref(),
                estimate.estimate_to.as_deref()
            ),
            (Some("3"), Some("5"))
        );
    }

    #[test]
    fn burndown_follows_additions_estimates_removals_and_completion() {
        let mut i = scn019();
        i.days = [
            ("2026-11-02", "2026-11-03T00:00:00Z"),
            ("2026-11-04", "2026-11-05T00:00:00Z"),
            ("2026-11-06", "2026-11-07T00:00:00Z"),
            ("2026-11-10", "2026-11-11T00:00:00Z"),
            ("2026-11-12", "2026-11-13T00:00:00Z"),
        ]
        .into_iter()
        .map(|(d, c)| (d.parse().unwrap(), at(c)))
        .collect();
        let figures = compute(&i);
        let points: Vec<_> = figures
            .burndown
            .iter()
            .map(|p| (p.day.map(|d| d.to_string()), p.remaining.value.clone()))
            .collect();
        assert_eq!(
            points,
            [
                (None, "8".to_owned()), // A 3 + B 5
                (Some("2026-11-02".into()), "8".into()),
                (Some("2026-11-04".into()), "12".into()), // C added (2), A raised to 5
                (Some("2026-11-06".into()), "7".into()),  // B removed
                (Some("2026-11-10".into()), "2".into()),  // A done
                (Some("2026-11-12".into()), "0".into()),  // C done
            ]
        );
        assert_eq!(figures.remaining.value, "0");
    }

    /// Example: a cancel lowers the burndown and never raises the completed amount; done before
    /// the start, canceled before the start, unestimated, 0 and another unit stay apart.
    #[test]
    fn the_remaining_work_at_the_close_is_what_was_left_not_nothing() {
        let mut i = scn019();
        let c = Uuid::from_u128(103);
        i.revs
            .insert(c, vec![rev("2026-11-03T00:00:00Z", TODO, Some(2))]);
        for assign in &mut i.assigns {
            if assign.ticket == c {
                assign.end_category = Some("NOT_STARTED".into());
            }
        }
        let figures = compute(&i);
        assert_eq!(
            (figures.remaining.value.as_str(), figures.remaining.tasks),
            ("2", 1)
        );
        assert_eq!(figures.completed.value, "5", "only A");
        let row = figures
            .tickets
            .iter()
            .find(|t| t.ticket.key == "C")
            .unwrap();
        assert_eq!(row.reason, "NOT_DONE");
    }

    #[test]
    fn scn020_cancel_zero_unestimated_and_before_the_start() {
        let mut i = input("POINTS");
        let ids: Vec<Uuid> = (1..=6).map(|n| Uuid::from_u128(100 + n)).collect();
        i.tickets = ids
            .iter()
            .enumerate()
            .map(|(n, id)| (*id, reference(n as u128 + 1, &format!("T{}", n + 1))))
            .collect();
        // T1 canceled in the Sprint, T2 done before the start, T3 canceled before the start,
        // T4 unestimated and done, T5 explicitly 0 and done, T6 written in hours and done.
        i.baseline = [
            (ids[0], base("IN_PROGRESS", Some(4))),
            (ids[1], base("DONE", Some(2))),
            (ids[2], base("CANCELED", Some(2))),
            (ids[3], base("NOT_STARTED", None)),
            (ids[4], base("NOT_STARTED", Some(0))),
            (
                ids[5],
                BaseRow {
                    category: "NOT_STARTED".into(),
                    estimate: Some((300, "HOURS".into())),
                },
            ),
        ]
        .into_iter()
        .collect();
        let hours = |when: &str, state: u128| Rev {
            at: at(when),
            status: Some(status(state)),
            estimate: Some((300, "HOURS".into())),
        };
        i.revs = [
            (
                ids[0],
                vec![
                    rev("2026-10-30T00:00:00Z", DOING, Some(4)),
                    rev("2026-11-05T00:00:00Z", CANCELED, Some(4)),
                ],
            ),
            (ids[1], vec![rev("2026-10-30T00:00:00Z", DONE, Some(2))]),
            (ids[2], vec![rev("2026-10-30T00:00:00Z", CANCELED, Some(2))]),
            (
                ids[3],
                vec![
                    rev("2026-10-30T00:00:00Z", TODO, None),
                    rev("2026-11-06T00:00:00Z", DONE, None),
                ],
            ),
            (
                ids[4],
                vec![
                    rev("2026-10-30T00:00:00Z", TODO, Some(0)),
                    rev("2026-11-06T00:00:00Z", DONE, Some(0)),
                ],
            ),
            (
                ids[5],
                vec![
                    hours("2026-10-30T00:00:00Z", TODO),
                    hours("2026-11-06T00:00:00Z", DONE),
                ],
            ),
        ]
        .into_iter()
        .collect();
        let end = "2026-11-13T09:00:00Z";
        i.assigns = vec![
            assign(
                1,
                "2026-10-31T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "CANCELED",
                Some(4),
            ),
            assign(
                2,
                "2026-10-31T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "DONE",
                Some(2),
            ),
            assign(
                3,
                "2026-10-31T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "CANCELED",
                Some(2),
            ),
            assign(
                4,
                "2026-10-31T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "DONE",
                None,
            ),
            assign(
                5,
                "2026-10-31T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "DONE",
                Some(0),
            ),
            Assign {
                end_estimate: Some((300, "HOURS".into())),
                ..assign(
                    6,
                    "2026-10-31T00:00:00Z",
                    Some(end),
                    Some("SPRINT_COMPLETED"),
                    "DONE",
                    None,
                )
            },
        ];
        let figures = compute(&i);
        // Planned: T1 4, T4 unestimated, T5 0, T6 other unit; T2 and T3 were finished before the start.
        assert_eq!(
            (
                figures.start_plan.value.as_str(),
                figures.start_plan.tasks,
                figures.start_plan.unestimated,
                figures.start_plan.other_unit
            ),
            ("4", 4, 1, 1)
        );
        // Completed: T4 (unestimated), T5 (0) and T6 (hours) are done in the Sprint; T1 canceled, T2 done before.
        assert_eq!(
            (
                figures.completed.value.as_str(),
                figures.completed.tasks,
                figures.completed.unestimated,
                figures.completed.other_unit
            ),
            ("0", 3, 1, 1)
        );
        let reason = |key: &str| {
            figures
                .tickets
                .iter()
                .find(|t| t.ticket.key == key)
                .unwrap()
                .reason
                .clone()
        };
        assert_eq!(
            (reason("T1"), reason("T2"), reason("T3"), reason("T4")),
            (
                "CANCELED".into(),
                "DONE_BEFORE".into(),
                "CANCELED".into(),
                "COUNTED".into()
            )
        );
        // The cancel lowers the burndown after it happens.
        let mut with_days = input("POINTS");
        with_days.tickets = i.tickets.clone();
        with_days.baseline = i.baseline.clone();
        with_days.revs = i.revs.clone();
        with_days.assigns = i.assigns.clone();
        with_days.days = vec![
            ("2026-11-03".parse().unwrap(), at("2026-11-04T00:00:00Z")),
            ("2026-11-05".parse().unwrap(), at("2026-11-06T00:00:00Z")),
        ];
        let points = compute(&with_days).burndown;
        assert_eq!(
            points[1].remaining.value, "4",
            "T1 still counts before it is canceled"
        );
        assert_eq!(
            points[2].remaining.value, "0",
            "a cancel lowers the remaining work"
        );
    }

    #[test]
    fn a_reopened_task_and_a_ticket_done_before_it_joined_are_not_completed() {
        let mut i = input("POINTS");
        let (a, b, c) = (
            Uuid::from_u128(101),
            Uuid::from_u128(102),
            Uuid::from_u128(103),
        );
        i.tickets = [
            (a, reference(1, "A")),
            (b, reference(2, "B")),
            (c, reference(3, "C")),
        ]
        .into_iter()
        .collect();
        i.baseline = [(a, base("NOT_STARTED", Some(3)))].into_iter().collect();
        let end = "2026-11-13T09:00:00Z";
        i.revs = [
            // A is done on the 5th and reopened on the 8th.
            (
                a,
                vec![
                    rev("2026-10-30T00:00:00Z", TODO, Some(3)),
                    rev("2026-11-05T00:00:00Z", DONE, Some(3)),
                    rev("2026-11-08T00:00:00Z", DOING, Some(3)),
                ],
            ),
            // B was done before it was added on the 6th.
            (b, vec![rev("2026-11-01T00:00:00Z", DONE, Some(2))]),
            // C is done, reopened, and done again inside the Sprint: its last DONE counts.
            (
                c,
                vec![
                    rev("2026-10-30T00:00:00Z", TODO, Some(1)),
                    rev("2026-11-03T00:00:00Z", DONE, Some(1)),
                    rev("2026-11-04T00:00:00Z", DOING, Some(1)),
                    rev("2026-11-09T00:00:00Z", DONE, Some(1)),
                ],
            ),
        ]
        .into_iter()
        .collect();
        i.assigns = vec![
            assign(
                1,
                "2026-10-31T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "IN_PROGRESS",
                Some(3),
            ),
            assign(
                2,
                "2026-11-06T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "DONE",
                Some(2),
            ),
            assign(
                3,
                "2026-10-31T00:00:00Z",
                Some(end),
                Some("SPRINT_COMPLETED"),
                "DONE",
                Some(1),
            ),
        ];
        let figures = compute(&i);
        let reason = |key: &str| {
            figures
                .tickets
                .iter()
                .find(|t| t.ticket.key == key)
                .unwrap()
                .reason
                .clone()
        };
        assert_eq!(
            (reason("A"), reason("B"), reason("C")),
            ("REOPENED".into(), "DONE_BEFORE".into(), "COUNTED".into())
        );
        assert_eq!(figures.completed.value, "1");
    }

    #[test]
    fn an_empty_plan_has_nothing_to_measure_and_the_unit_none_counts_tasks() {
        let mut i = input("NONE");
        let a = Uuid::from_u128(101);
        i.tickets = [(a, reference(1, "A"))].into_iter().collect();
        i.revs = [(
            a,
            vec![
                rev("2026-11-03T00:00:00Z", TODO, None),
                rev("2026-11-05T00:00:00Z", DONE, None),
            ],
        )]
        .into_iter()
        .collect();
        i.assigns = vec![assign(
            1,
            "2026-11-03T12:00:00Z",
            Some("2026-11-13T09:00:00Z"),
            Some("SPRINT_COMPLETED"),
            "DONE",
            None,
        )];
        let figures = compute(&i);
        assert!(figures.adherence.not_applicable && figures.adherence.percent.is_none());
        assert_eq!(figures.start_plan.value, "0");
        assert_eq!(
            (figures.completed.value.as_str(), figures.completed.tasks),
            ("1", 1),
            "with no unit a completed Task counts as 1"
        );
    }

    #[test]
    fn an_active_sprint_reads_the_timeline_up_to_now() {
        let mut i = scn019();
        i.active = true;
        i.ended_at = None;
        i.now = at("2026-11-09T00:00:00Z");
        for assign in &mut i.assigns {
            if assign.reason.as_deref() == Some("SPRINT_COMPLETED") {
                assign.to = None;
                assign.reason = None;
                assign.end_category = None;
                assign.end_estimate = None;
            }
        }
        let figures = compute(&i);
        // On the 9th A (5) is still open and C (2) is open; B was removed on the 6th.
        assert_eq!(figures.remaining.value, "7");
        assert_eq!(figures.completed.value, "0");
        let a = figures
            .tickets
            .iter()
            .find(|t| t.ticket.key == "A")
            .unwrap();
        assert_eq!(a.reason, "OPEN");
    }

    #[test]
    fn category_history_is_read_at_the_time_not_through_the_current_workflow() {
        // State 2 was IN_PROGRESS until the 5th and DONE after: a revision on the 3rd is open, one on the 8th is done.
        let mut history = History::new();
        history.insert(
            status(2),
            vec![
                (None, "IN_PROGRESS".into()),
                (Some(at("2026-11-05T00:00:00Z")), "DONE".into()),
            ],
        );
        let revs = vec![Rev {
            at: at("2026-11-01T00:00:00Z"),
            status: Some(status(2)),
            estimate: None,
        }];
        assert_eq!(
            category_at(&revs, &history, at("2026-11-03T00:00:00Z")),
            Some("IN_PROGRESS")
        );
        assert_eq!(
            category_at(&revs, &history, at("2026-11-08T00:00:00Z")),
            Some("DONE")
        );
        assert_eq!(
            category_at(&revs, &history, at("2026-10-01T00:00:00Z")),
            None,
            "before the Ticket existed"
        );
    }
}

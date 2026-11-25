//! What MCP never does directly, and where a person does it instead (FEA-172).
//!
//! Three kinds of action live here: account security and device registration (a person types a
//! secret or grants a browser permission), privilege changes (OWNER/ADMIN roles), and the actions the
//! web app protects with a recent sign-in. For the last kind the check is in the HTTP layer (the
//! `RecentActor` extractor wraps `core` functions that take only a user id), so calling the `core`
//! function from an MCP operation would skip it. An OAuth consent, a client's tool approval or a
//! `*:manage` scope is not a recent sign-in. A test (`web_only_covers_every_recent_actor_route`)
//! reads the HTTP sources and fails when a handler with `RecentActor` is missing from this list;
//! another fails when an operation carries one of the names below.

pub struct Entry {
    /// What a person asks for.
    pub action: &'static str,
    /// HTTP handler functions that guard it with `RecentActor` (checked against the sources).
    pub handlers: &'static [&'static str],
    pub why: &'static str,
    pub where_in_web: &'static str,
}

pub const ENTRIES: &[Entry] = &[
    Entry {
        action: "Change password, e-mail, MFA, recovery codes; link or unlink an external sign-in; recover an account",
        handlers: &[],
        why: "A person types secrets or one-time codes; an AI must never see them.",
        where_in_web: "Profile > Security",
    },
    Entry {
        action: "Register a browser or device for Web Push",
        handlers: &[],
        why: "The browser asks the person for its own permission.",
        where_in_web: "Profile > Notifications",
    },
    Entry {
        action: "Approve an OAuth client or an added scope; revoke all MCP access at once",
        handlers: &["revoke_all_grants"],
        why: "Consent and revocation are the person's own security decisions.",
        where_in_web: "Profile > Connected apps",
    },
    Entry {
        action: "Change the Workspace MCP policy",
        handlers: &[],
        why: "Needs a recent sign-in; it decides what MCP may do at all.",
        where_in_web: "Workspace settings > MCP",
    },
    Entry {
        action: "Make someone OWNER or ADMIN, take those roles away, or invite someone with those roles",
        handlers: &["change_member_role"],
        why: "A privilege escalation; the web app asks for a recent sign-in.",
        where_in_web: "Workspace settings > Members",
    },
    Entry {
        action: "Remove a member by force, disable or enable an account",
        handlers: &["remove_member", "set_account_disabled"],
        why: "Needs a recent sign-in in the web app.",
        where_in_web: "Workspace settings > Members",
    },
    Entry {
        action: "Recover a Project, Doc or Channel that has no manager",
        handlers: &[
            "recover_project_admin",
            "recover_doc_manager",
            "recover_channel_manager",
        ],
        why: "Gives the person access they did not have; needs a recent sign-in.",
        where_in_web: "Workspace settings > Recovery",
    },
    Entry {
        action: "Delete a Ticket permanently",
        handlers: &["purge_ticket"],
        why: "Irreversible; needs a recent sign-in. Trash and restore are available in MCP.",
        where_in_web: "Project > Trash",
    },
    Entry {
        action: "Create, rotate or revoke a Channel webhook",
        handlers: &["create_webhook", "rotate_webhook", "revoke_webhook"],
        why: "Handles a secret; needs a recent sign-in. Listing webhooks is available in MCP.",
        where_in_web: "Channel settings > Webhooks",
    },
];

/// Operation ids that must never exist: each is a web-only action. Checked by a test.
pub const FORBIDDEN_OPERATIONS: &[&str] = &[
    "member.set_role",
    "member.change_role",
    "member.remove",
    "member.disable",
    "member.enable",
    "invitation.create_privileged",
    "policy.set",
    "workspace.set_mcp_policy",
    "ticket.purge",
    "webhook.create",
    "webhook.rotate",
    "webhook.revoke",
    "project.admin_recovery",
    "doc.manager_recovery",
    "channel.manager_recovery",
    "grants.revoke_all",
    "password.change",
    "mfa.enable",
    "mfa.disable",
    "push.subscribe",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Source files of the HTTP layer that hold handlers.
    const SOURCES: &[(&str, &str)] = &[
        ("admin.rs", include_str!("../../http-api/src/admin.rs")),
        (
            "mcp_routes.rs",
            include_str!("../../http-api/src/mcp_routes.rs"),
        ),
        (
            "routes/webhooks.rs",
            include_str!("../../http-api/src/routes/webhooks.rs"),
        ),
        (
            "routes/files.rs",
            include_str!("../../http-api/src/routes/files.rs"),
        ),
        (
            "routes/tickets.rs",
            include_str!("../../http-api/src/routes/tickets.rs"),
        ),
        (
            "routes/project.rs",
            include_str!("../../http-api/src/routes/project.rs"),
        ),
        (
            "routes/organization.rs",
            include_str!("../../http-api/src/routes/organization.rs"),
        ),
        (
            "routes/workspace.rs",
            include_str!("../../http-api/src/routes/workspace.rs"),
        ),
        (
            "routes/conversation.rs",
            include_str!("../../http-api/src/routes/conversation.rs"),
        ),
        (
            "routes/personal.rs",
            include_str!("../../http-api/src/routes/personal.rs"),
        ),
        (
            "routes/planning.rs",
            include_str!("../../http-api/src/routes/planning.rs"),
        ),
        (
            "knowledge.rs",
            include_str!("../../http-api/src/knowledge.rs"),
        ),
    ];

    /// Handler functions whose signature takes the `RecentActor` extractor.
    fn recent_actor_handlers() -> Vec<(String, String)> {
        let mut found = Vec::new();
        for (file, text) in SOURCES {
            for chunk in text.split("async fn ").skip(1) {
                let name: String = chunk
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let signature = chunk.split(") ->").next().unwrap_or("");
                if signature.contains("RecentActor(") {
                    found.push((file.to_string(), name));
                }
            }
        }
        found
    }

    #[test]
    fn web_only_covers_every_recent_actor_route() {
        let listed: Vec<&str> = ENTRIES
            .iter()
            .flat_map(|e| e.handlers.iter().copied())
            .collect();
        let found = recent_actor_handlers();
        assert!(!found.is_empty(), "the scan found no RecentActor handler");
        for (file, name) in &found {
            assert!(
                listed.contains(&name.as_str()),
                "{file}: {name} needs a recent sign-in in the web app but is missing from web_only::ENTRIES; do not expose it through MCP"
            );
        }
        // A listed handler that no longer exists points at a stale entry. (`change_member_role` is
        // checked by name: its recent-sign-in rule applies to OWNER/ADMIN changes only.)
        for name in listed {
            let in_sources = SOURCES
                .iter()
                .any(|(_, text)| text.contains(&format!("async fn {name}(")));
            assert!(
                in_sources,
                "web_only::ENTRIES names {name}, which is not a handler"
            );
        }
    }

    #[test]
    fn no_operation_is_a_web_only_action() {
        for op in crate::catalog::all() {
            assert!(
                !FORBIDDEN_OPERATIONS.contains(&op.id),
                "{} is a web-only action",
                op.id
            );
        }
    }
}

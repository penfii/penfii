//! Telling the open pages of a Doc that it changed.
//!
//! Every way a page changes (the editor's Save, an MCP or API write, the co-editing layer's materialization, the
//! title, the width, who may read it, a Version, the deletion) already records an event. This consumer turns the
//! ones a reader of the page can see into a [`ChangeNotice::Doc`], so one rule covers every writer. The notice
//! carries no content and no names: the socket of an open page checks, for each person, that they may still read the
//! page and tells their client to read it again; what is read is decided by the same API as any other read.

use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::events::{EventRow, ExternalConsumer, HandlerFuture};
use crate::platform::ChangeNotice;

/// The event kinds that change what an open page shows. Comments, references and moves are not among them: a page
/// has its own live reading of comments, and the tree is not part of the page.
const KINDS: &[&str] = &[
    "doc.updated",
    "doc.layout_changed",
    "doc.visibility_changed",
    "doc.participants_changed",
    "doc.version_created",
    "doc.deleted",
];

pub struct DocLiveConsumer {
    changes: broadcast::Sender<ChangeNotice>,
}

impl DocLiveConsumer {
    pub fn new(changes: broadcast::Sender<ChangeNotice>) -> Self {
        Self { changes }
    }
}

impl ExternalConsumer for DocLiveConsumer {
    fn name(&self) -> &'static str {
        "doc_live"
    }

    fn accepts(&self, event_kind: &str) -> bool {
        KINDS.contains(&event_kind)
    }

    /// Publishing twice is harmless (a client that is told twice reads once more), so a redelivered event needs no memory.
    fn handle<'a>(&'a self, _pool: &'a PgPool, event: &'a EventRow) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Every `doc.*` event targets its page. Nobody listening is not an error.
            let _ = self.changes.send(ChangeNotice::Doc {
                page_id: event.target_id,
            });
            Ok(())
        })
    }
}

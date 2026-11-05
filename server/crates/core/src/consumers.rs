//! The fixed list of event consumers. There is no plug-in mechanism: adding a consumer means
//! adding it here.

use crate::events::Registry;
use crate::search::SearchEngine;

pub fn registry(
    search: Option<&SearchEngine>,
    changes: Option<tokio::sync::broadcast::Sender<crate::ChangeNotice>>,
) -> Registry {
    let mut registry = Registry::default();
    registry
        .pg
        .push(std::sync::Arc::new(crate::notify::WatchConsumer));
    registry.pg.push(crate::notify::InboxConsumer::new(changes));
    if let Some(engine) = search {
        registry.external.push(crate::search::consumer(engine));
    }
    registry
}

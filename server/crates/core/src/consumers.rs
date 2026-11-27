//! The fixed list of event consumers. There is no plug-in mechanism: adding a consumer means
//! adding it here.

use crate::events::Registry;
use crate::search::SearchEngine;

pub fn registry(
    search: Option<&SearchEngine>,
    changes: Option<tokio::sync::broadcast::Sender<crate::ChangeNotice>>,
) -> Registry {
    registry_with_push(search, changes, None, None)
}

/// `registry`, with Web Push for the Inbox kinds that are pushed (see `push::PUSH_KINDS`).
/// `store` registers the consumer that deletes a file's object after the file is retired.
pub fn registry_with_push(
    search: Option<&SearchEngine>,
    changes: Option<tokio::sync::broadcast::Sender<crate::ChangeNotice>>,
    push: Option<std::sync::Arc<crate::push::PushService>>,
    store: Option<crate::files::ObjectStore>,
) -> Registry {
    let mut registry = Registry::default();
    registry
        .pg
        .push(std::sync::Arc::new(crate::notify::WatchConsumer));
    registry.pg.push(crate::notify::InboxConsumer::with_push(
        changes.clone(),
        push,
    ));
    // Open pages of a Doc read it again when it changes; without a broadcast there is nobody to tell.
    if let Some(sender) = changes.clone() {
        registry.external.push(std::sync::Arc::new(
            crate::docs::live::DocLiveConsumer::new(sender),
        ));
    }
    if let Some(engine) = search {
        registry.external.push(crate::search::consumer(engine));
    }
    if let Some(store) = store {
        registry
            .external
            .push(crate::files::storage_consumer(store));
    }
    registry
}

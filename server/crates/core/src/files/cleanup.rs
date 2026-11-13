//! Deletes the object of a file after the database has committed that the file is gone.
//!
//! The bucket is assumed not to use S3 Versioning. A delete removes the current object; a
//! versioned bucket can keep older versions, and this consumer does not address those.
//! Delivery is at least once: deleting an object that is already gone succeeds, and an object is
//! deleted only when the file row still says `deleted_at` is set. The key comes from that row,
//! not from the event.

use std::sync::Arc;

use sqlx::PgPool;

use super::ObjectStore;
use crate::error::CoreError;
use crate::events::{EventRow, ExternalConsumer, HandlerFuture};

pub struct StorageConsumer {
    store: ObjectStore,
}

pub fn consumer(store: ObjectStore) -> Arc<dyn ExternalConsumer> {
    Arc::new(StorageConsumer { store })
}

impl ExternalConsumer for StorageConsumer {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn accepts(&self, kind: &str) -> bool {
        kind == "storage.file_delete"
    }

    fn handle<'a>(&'a self, pool: &'a PgPool, event: &'a EventRow) -> HandlerFuture<'a> {
        let store = self.store.clone();
        Box::pin(async move {
            let row: Option<(bool, String)> = sqlx::query_as(
                "SELECT deleted_at IS NOT NULL, storage_key FROM penfii.files WHERE id = $1",
            )
            .bind(event.target_id)
            .fetch_optional(pool)
            .await?;
            // A missing row, or a file that is not deleted, is nothing to do. The event must not
            // remove an object that a live file still names.
            let Some((deleted, key)) = row else {
                return Ok(());
            };
            if !deleted {
                return Ok(());
            }
            store.delete(&key).await.map_err(CoreError::Internal)?;
            Ok(())
        })
    }
}

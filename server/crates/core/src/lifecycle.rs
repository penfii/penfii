//! The stop signal of the process. The background workers (event consumers, reminders, the Sprint
//! archive, retention and sweepers, search maintenance) loop until [`stopping`] and wait with
//! [`pause`], which returns at once when the stop is requested. `core-api` asks for the stop after the
//! HTTP server has finished and before the co-editing hub and the database pool are shut down, so a
//! worker finishes the unit of work it is in (one transaction or one idempotent step) and does not
//! start another.

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::FutureExt;
use tokio::sync::watch;

fn channel() -> &'static watch::Sender<bool> {
    static STOP: OnceLock<watch::Sender<bool>> = OnceLock::new();
    STOP.get_or_init(|| watch::channel(false).0)
}

/// Asks every background worker to stop after the work it is doing.
pub fn request_stop() {
    channel().send_replace(true);
}

/// Whether the stop was requested.
pub fn stopping() -> bool {
    *channel().borrow()
}

/// Sleeps for `duration`, or less when the stop is requested meanwhile.
pub async fn pause(duration: Duration) {
    let mut stop = channel().subscribe();
    if *stop.borrow_and_update() {
        return;
    }
    tokio::select! {
        _ = tokio::time::sleep(duration) => {}
        _ = stop.changed() => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Worker failures
//
// A panic inside one unit of work (one event, one schedule, one pass of a sweeper) is caught where the
// unit runs ([`contained`]) and counted as that unit's failure: the worker goes on, and the existing
// retry, attempt and DEAD rules apply. A panic that gets out of a worker's loop (nothing caught it)
// is not expected: the process records why and ends ([`spawn_worker`], [`fatal`]), and the container
// runtime or another supervisor starts it again. Nothing here restarts a worker inside the process.
// ---------------------------------------------------------------------------------------------

/// The text of a panic payload (`&str` and `String` carry one; anything else has none).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "panic without a message".to_owned()
    }
}

/// Runs `work` and turns a panic in it into `Err(message)`. Use it around one unit of work whose failure
/// is already handled by the caller (retry, skip, next pass). Anything the unit held (a transaction, a
/// connection) is dropped by the unwind and rolled back; do not reuse a transaction the unit was using
/// unless the caller knows it is still sound.
pub async fn contained<F, T>(work: F) -> Result<T, String>
where
    F: Future<Output = T>,
{
    AssertUnwindSafe(work)
        .catch_unwind()
        .await
        .map_err(|payload| panic_message(&*payload))
}

/// [`contained`] for one pass of a worker whose function answers `Result<_, CoreError>`: a panic is logged
/// as an error with the worker's name and becomes the pass's `Err`, so the worker handles it like any
/// other failed pass (a warning, then the next pass).
pub async fn guarded<F, T>(name: &'static str, pass: F) -> Result<T, crate::error::CoreError>
where
    F: Future<Output = Result<T, crate::error::CoreError>>,
{
    match contained(pass).await {
        Ok(result) => result,
        Err(message) => {
            tracing::error!(worker = name, %message, "a pass of the worker panicked");
            Err(crate::error::CoreError::Internal(format!(
                "panic: {message}"
            )))
        }
    }
}

/// What happens when a worker's loop panics. The argument is the worker's name and the panic message.
pub type OnFatal = Arc<dyn Fn(&'static str, &str) + Send + Sync>;

fn fatal_channel() -> &'static watch::Sender<Option<String>> {
    static FATAL: OnceLock<watch::Sender<Option<String>>> = OnceLock::new();
    FATAL.get_or_init(|| watch::channel(None).0)
}

/// Records that the process must end because a worker's loop panicked. The first reason is kept.
pub fn request_fatal(worker: &'static str, message: &str) {
    fatal_channel().send_if_modified(|reason| {
        if reason.is_some() {
            return false;
        }
        *reason = Some(format!("{worker}: {message}"));
        true
    });
}

/// Resolves with the reason once [`request_fatal`] was called (`core-api` waits for it next to the stop signal).
pub async fn fatal() -> String {
    let mut reason = fatal_channel().subscribe();
    loop {
        if let Some(found) = reason.borrow_and_update().clone() {
            return found;
        }
        if reason.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Starts a long-lived worker. When its future panics (outside any [`contained`] unit of work) and the
/// process is not already stopping, the panic is logged as an error and the process is asked to end
/// ([`request_fatal`]).
pub fn spawn_worker<F>(name: &'static str, work: F) -> tokio::task::JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    spawn_worker_with(name, work, Arc::new(request_fatal))
}

/// [`spawn_worker`] with the reaction to a panic given. Tests use it to see that the reaction is called
/// without ending the test process.
pub fn spawn_worker_with<F>(
    name: &'static str,
    work: F,
    on_fatal: OnFatal,
) -> tokio::task::JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let Err(message) = contained(work).await else {
            return;
        };
        if stopping() {
            tracing::error!(worker = name, %message, "worker panicked while the process was stopping");
        } else {
            tracing::error!(worker = name, %message, "worker panicked: the process ends so that it is started again");
            on_fatal(name, &message);
        }
    })
}

/// Logs every panic of the process with its place, through `tracing` (the default hook writes to stderr
/// only). The earlier hook still runs.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_default();
        tracing::error!(%place, message = %panic_message(info.payload()), "panic");
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn contained_and_guarded_turn_a_panic_into_a_failure() {
        let caught = contained(async { panic!("one event") }).await;
        assert_eq!(caught.unwrap_err(), "one event");
        assert_eq!(contained(async { 7 }).await, Ok(7));
        let pass: Result<(), _> = guarded("test", async { panic!("a pass {}", 2) }).await;
        match pass {
            Err(crate::error::CoreError::Internal(text)) => assert_eq!(text, "panic: a pass 2"),
            other => panic!("{other:?}"),
        }
    }

    /// One test, in this order: the stop request is for the whole process and cannot be taken back.
    #[tokio::test]
    async fn a_worker_that_panics_ends_the_process_unless_it_is_already_stopping() {
        let reactions: Arc<Mutex<Vec<(&'static str, String)>>> = Arc::default();
        let record = {
            let reactions = reactions.clone();
            Arc::new(move |name: &'static str, message: &str| {
                reactions.lock().unwrap().push((name, message.to_owned()));
            }) as OnFatal
        };

        // A worker that ends normally is not a failure.
        spawn_worker_with("quiet", async {}, record.clone())
            .await
            .unwrap();
        assert!(reactions.lock().unwrap().is_empty());

        // A panic out of the loop is: the reaction gets the worker's name and the message.
        spawn_worker_with("loud", async { panic!("loop broke") }, record.clone())
            .await
            .unwrap();
        assert_eq!(
            *reactions.lock().unwrap(),
            vec![("loud", "loop broke".to_owned())]
        );

        // While the process is stopping, a panic is logged and the reaction is not called.
        request_stop();
        spawn_worker_with("late", async { panic!("while stopping") }, record)
            .await
            .unwrap();
        assert_eq!(reactions.lock().unwrap().len(), 1);

        // The first reason is the one kept.
        request_fatal("first", "a");
        request_fatal("second", "b");
        assert_eq!(fatal().await, "first: a");
    }
}

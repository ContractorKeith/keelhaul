//! Convert worker failures into the same cancellation path as ordinary I/O errors.
use super::pipeline::Event;
use crate::{
    backend::{Backend, BackendError},
    repo::{Error, Repository, blobs::Upload},
};
use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        mpsc::{Receiver, SyncSender},
    },
};
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown panic payload".into())
}
pub(super) fn run(
    stage: &str,
    errors: SyncSender<Result<Event, Error>>,
    action: impl FnOnce() -> Result<(), Error>,
) {
    let result = catch_unwind(AssertUnwindSafe(action)).unwrap_or_else(|panic| {
        Err(Error::Refused(format!(
            "{stage} worker panicked: {}",
            panic_message(panic)
        )))
    });
    match result {
        Err(error) => {
            // Make the injected EMFILE regression exercise cancellation ordering.
            #[cfg(feature = "fault-injection")]
            if stage == "file" && std::env::var_os("KEELHAUL_TEST_EMFILE_AT_OPEN").is_some() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let _ = errors.send(Err(error));
        }
        Ok(()) => {
            let _ = errors.send(Ok(Event::Finished(stage == "upload")));
        }
    }
}
pub(super) fn receive<T>(receiver: &Mutex<Receiver<T>>, stage: &str) -> Result<Option<T>, Error> {
    let receiver = receiver
        .lock()
        .map_err(|_| Error::Refused(format!("{stage} queue mutex poisoned")))?;
    Ok(receiver.recv().ok())
}
pub(super) fn upload<B: Backend>(repo: &Repository<B>, task: &Upload) -> Result<(), BackendError> {
    catch_unwind(AssertUnwindSafe(|| {
        repo.backend.write(&task.name, &task.bytes)
    }))
    .unwrap_or_else(|panic| {
        Err(
            std::io::Error::other(format!("upload worker panicked: {}", panic_message(panic)))
                .into(),
        )
    })
    .map_err(|e| e.context(Some(&task.name), None))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};
    #[test]
    fn worker_panic_is_reported_instead_of_escaping_scope() {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            scope.spawn(|| run("file", sender, || panic!("injected panic")));
        });
        let error = receiver.recv().unwrap().err().unwrap();
        assert!(
            error
                .to_string()
                .contains("file worker panicked: injected panic")
        );
    }
    #[test]
    fn poisoned_queue_is_reported_on_events_channel() {
        let (_, receiver) = mpsc::sync_channel::<()>(1);
        let receiver = Arc::new(Mutex::new(receiver));
        let poison = receiver.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = poison.lock().unwrap();
                panic!("poison");
            })
            .join()
            .is_err()
        );
        let (sender, errors) = mpsc::sync_channel(1);
        run("file", sender, || receive(&receiver, "file").map(|_| ()));
        assert!(
            errors
                .recv()
                .unwrap()
                .err()
                .unwrap()
                .to_string()
                .contains("queue mutex poisoned")
        );
    }
}

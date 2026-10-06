//! Runs blocking work that must happen on the process's main thread.
//!
//! macOS only shows native dialogs on the main thread when no app event loop
//! is running (our server is a background process), so there the server runs
//! on a side thread and the main thread waits for jobs. Other systems run the
//! server on the main thread and the work on a blocking worker thread.

/// Runs the server (`run` never returns while it serves), keeping the main
/// thread free for [`run`] jobs where the system needs that.
pub fn serve(run: impl FnOnce() + Send + 'static) {
    #[cfg(target_os = "macos")]
    imp::serve(run);

    #[cfg(not(target_os = "macos"))]
    run();
}

/// Runs `f` off the async runtime: on the main thread on macOS, on a blocking
/// worker elsewhere.
pub async fn run<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Result<R, String> {
    #[cfg(target_os = "macos")]
    return imp::run(f).await;

    #[cfg(not(target_os = "macos"))]
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    type Job = Box<dyn FnOnce() + Send>;

    static QUEUE: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

    pub fn serve(run: impl FnOnce() + Send + 'static) {
        let (tx, rx) = channel::<Job>();
        let _ = QUEUE.set(Mutex::new(tx));
        let server = std::thread::spawn(run);

        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(job) => job(),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // The server stopped (or panicked): don't keep the process alive.
            if server.is_finished() {
                break;
            }
        }
        if let Err(panic) = server.join() {
            std::panic::resume_unwind(panic);
        }
    }

    pub async fn run<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Result<R, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });
        QUEUE
            .get()
            .ok_or("main thread isn't taking jobs")?
            .lock()
            .map_err(|e| e.to_string())?
            .send(job)
            .map_err(|_| "main thread stopped")?;
        rx.await.map_err(|_| "main thread job failed".to_string())
    }
}

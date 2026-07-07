use speare::Error;
use speare::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    sync::Notify,
    time::{Duration, timeout},
};

#[tokio::test]
async fn task_can_request_app_wide_shutdown() -> Result<(), Error> {
    // Arrange
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .build();

    app.spawn({
        let cancelled = cancelled.clone();

        move |scope: Scope| {
            let cancelled = cancelled.clone();

            async move {
                scope.cancelled().await;
                cancelled.store(true, Ordering::SeqCst);
            }
        }
    });
    app.spawn(|shutdown: Shutdown| async move {
        shutdown.now();
    });

    // Act
    app.join().await?;

    // Assert
    assert!(cancelled.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn app_shutdown_cancels_running_root_tasks() -> Result<(), Error> {
    // Arrange
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .build();

    app.spawn({
        let cancelled = cancelled.clone();

        move |scope: Scope| {
            let cancelled = cancelled.clone();

            async move {
                scope.cancelled().await;
                cancelled.store(true, Ordering::SeqCst);
            }
        }
    });

    // Act
    app.shutdown();
    app.join().await?;

    // Assert
    assert!(cancelled.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn shutdown_aborts_root_task_after_cancel_timeout() -> Result<(), Error> {
    // Arrange
    let started = Arc::new(Notify::new());
    let aborted = Arc::new(Notify::new());
    let mut app = App::builder()
        .cancel_timeout(Duration::from_millis(10))
        .build();

    app.spawn({
        let started = started.clone();
        let aborted = aborted.clone();

        move || {
            let started = started.clone();
            let aborted = aborted.clone();

            async move {
                struct AbortGuard(Arc<Notify>);

                impl Drop for AbortGuard {
                    fn drop(&mut self) {
                        self.0.notify_one();
                    }
                }

                let _guard = AbortGuard(aborted);
                started.notify_one();
                std::future::pending::<()>().await;
            }
        }
    });

    started.notified().await;

    // Act
    app.shutdown();
    app.join().await?;

    // Assert
    timeout(Duration::from_secs(1), aborted.notified())
        .await
        .unwrap();
    Ok(())
}

#[tokio::test]
async fn defer_runs_after_task_requested_shutdown() -> Result<(), Error> {
    // Arrange
    let cleaned_up = Arc::new(AtomicBool::new(false));
    let mut app = App::new();

    app.defer({
        let cleaned_up = cleaned_up.clone();

        move || {
            let cleaned_up = cleaned_up.clone();

            async move {
                cleaned_up.store(true, Ordering::SeqCst);
            }
        }
    });
    app.spawn(|shutdown: Shutdown| async move {
        shutdown.now();
    });

    // Act
    app.join().await?;

    // Assert
    assert!(cleaned_up.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn root_failure_error_wins_over_defer_error() {
    // Arrange
    let mut app = App::new();

    app.defer(|| async { Err::<(), _>(Error::source(std::io::Error::other("defer failed"))) });
    app.spawn(|| async { Err::<(), _>(Error::source(std::io::Error::other("root failed"))) });

    // Act
    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
}

use speare::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

#[tokio::test]
async fn spawn_runs_root_task_and_join_waits_for_completion() -> Result<()> {
    // Arrange
    let ran = Arc::new(AtomicBool::new(false));
    let mut app = App::builder()
        .cancel_timeout(Duration::from_millis(50))
        .build();

    // Act
    app.spawn({
        let ran = ran.clone();
        move || {
            let ran = ran.clone();

            async move {
                ran.store(true, Ordering::SeqCst);
            }
        }
    });
    app.join().await?;

    // Assert
    assert!(ran.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn spawn_ignores_successful_task_value() -> Result<()> {
    // Arrange
    let mut app = App::new();

    // Act
    app.spawn(|| async { Ok::<_, Error>(42i32) });
    let result = app.join().await;

    // Assert
    assert!(result.is_ok());
    Ok(())
}

#[tokio::test]
async fn root_task_failure_cancels_sibling_and_join_returns_error() {
    // Arrange
    let sibling_cancelled = Arc::new(AtomicBool::new(false));
    let mut app = App::new();

    // Act
    app.spawn({
        let sibling_cancelled = sibling_cancelled.clone();
        move |scope: Scope| {
            let sibling_cancelled = sibling_cancelled.clone();

            async move {
                scope.cancelled().await;
                sibling_cancelled.store(true, Ordering::SeqCst);
            }
        }
    });
    app.spawn(|| async { Err::<(), _>(Error::source(std::io::Error::other("root failed"))) });

    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
    assert!(sibling_cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn root_task_retry_is_exhausted_before_join_returns_error() {
    // Arrange
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut app = App::new();

    // Act
    app.spawn(
        {
            let attempts = attempts.clone();
            move || {
                let attempts = attempts.clone();

                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(Error::source(std::io::Error::other("retry failed")))
                }
            }
        }
        .retry(Retry::Max(2)),
    );

    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "retry failed");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn app_can_spawn_and_join_again_after_successful_join() -> Result<()> {
    // Arrange
    let runs = Arc::new(AtomicUsize::new(0));
    let mut app = App::new();

    // Act
    app.spawn({
        let runs = runs.clone();
        move || {
            let runs = runs.clone();

            async move {
                runs.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    app.join().await?;

    app.spawn({
        let runs = runs.clone();
        move || {
            let runs = runs.clone();

            async move {
                runs.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    app.join().await?;

    // Assert
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn app_resets_after_failed_join_and_can_run_new_root_tasks() -> Result<()> {
    // Arrange
    let ran_after_failure = Arc::new(AtomicBool::new(false));
    let mut app = App::new();

    // Act
    app.spawn(|| async { Err::<(), _>(Error::source(std::io::Error::other("root failed"))) });
    let err = app.join().await.unwrap_err();

    app.spawn({
        let ran_after_failure = ran_after_failure.clone();
        move || {
            let ran_after_failure = ran_after_failure.clone();

            async move {
                ran_after_failure.store(true, Ordering::SeqCst);
            }
        }
    });
    app.join().await?;

    // Assert
    assert_eq!(err.to_string(), "root failed");
    assert!(ran_after_failure.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn shutdown_requests_app_wide_cancellation_without_waiting() -> Result<()> {
    // Arrange
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut app = App::new();

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
async fn task_spawned_after_shutdown_observes_cancellation_immediately() -> Result<()> {
    // Arrange
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut app = App::new();

    // Act
    app.shutdown();
    app.spawn({
        let cancelled = cancelled.clone();
        move |scope: Scope| {
            let cancelled = cancelled.clone();

            async move {
                if scope.is_cancelled() {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
        }
    });
    app.join().await?;

    // Assert
    assert!(cancelled.load(Ordering::SeqCst));
    Ok(())
}

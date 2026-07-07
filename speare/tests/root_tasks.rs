use speare::Error;
use speare::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Default)]
struct Flag(AtomicBool);

#[derive(Default)]
struct Count(AtomicUsize);

#[tokio::test]
async fn spawn_runs_root_task_and_join_waits_for_completion() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder()
        .cancel_timeout(Duration::from_millis(50))
        .insert_resource(Flag::default())
        .build();

    // Act
    app.spawn(async |ran: Res<Flag>| {
        ran.0.store(true, Ordering::SeqCst);
    });

    app.join().await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));

    Ok(())
}

#[tokio::test]
async fn spawn_ignores_successful_task_value() -> Result<(), Error> {
    // Arrange
    let mut app = App::new();

    // Act
    app.spawn(async || -> Result<_, Error> { Ok(42i32) });
    let result = app.join().await;

    // Assert
    assert!(result.is_ok());
    Ok(())
}

#[tokio::test]
async fn root_task_failure_cancels_sibling_and_join_returns_error() {
    // Arrange
    let mut app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.spawn(async |scope: Scope, sibling_cancelled: Res<Flag>| {
        scope.cancelled().await;
        sibling_cancelled.0.store(true, Ordering::SeqCst);
    })
    .spawn(async || -> Result<(), _> { Err("root failed") });

    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
    assert!(app.get::<Flag>().unwrap().0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn root_task_retry_is_exhausted_before_join_returns_error() {
    // Arrange
    let mut app = App::builder().insert_resource(Count::default()).build();
    let inc_attempts = async |attempts: Res<Count>| -> Result<(), _> {
        attempts.0.fetch_add(1, Ordering::SeqCst);
        Err("retry failed")
    };

    // Act
    app.spawn(inc_attempts.retry(Retry::Max(2)));
    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "retry failed");
    assert_eq!(app.get::<Count>().unwrap().0.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn app_can_spawn_and_join_again_after_successful_join() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder().insert_resource(Count::default()).build();
    let inc_runs = async |runs: Res<Count>| {
        runs.0.fetch_add(1, Ordering::SeqCst);
    };

    // Act
    app.spawn(inc_runs);
    app.join().await?;

    app.spawn(inc_runs);
    app.join().await?;

    // Assert
    assert_eq!(app.get::<Count>()?.0.load(Ordering::SeqCst), 2);

    Ok(())
}

#[tokio::test]
async fn app_resets_after_failed_join_and_can_run_new_root_tasks() -> Result<(), Error> {
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
async fn shutdown_requests_app_wide_cancellation_without_waiting() -> Result<(), Error> {
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
async fn task_spawned_after_shutdown_observes_cancellation_immediately() -> Result<(), Error> {
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

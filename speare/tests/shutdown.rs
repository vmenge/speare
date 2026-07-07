use speare::Error;
use speare::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::{
    sync::Notify,
    time::{Duration, timeout},
};

#[derive(Default)]
struct Flag(AtomicBool);

#[derive(Default)]
struct Started(Notify);

#[derive(Default)]
struct Aborted(Notify);

struct AbortGuard(Res<Aborted>);

impl Drop for AbortGuard {
    fn drop(&mut self) {
        self.0.0.notify_one();
    }
}

#[tokio::test]
async fn task_can_request_app_wide_shutdown() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .insert_resource(Flag::default())
        .build();

    // Act
    app.spawn(async |scope: Scope, cancelled: Res<Flag>| {
        scope.cancelled().await;
        cancelled.0.store(true, Ordering::SeqCst);
    })
    .spawn(|shutdown: Shutdown| async move {
        shutdown.now();
    });

    app.join().await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn app_shutdown_cancels_running_root_tasks() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .insert_resource(Flag::default())
        .build();

    app.spawn(async |scope: Scope, cancelled: Res<Flag>| {
        scope.cancelled().await;
        cancelled.0.store(true, Ordering::SeqCst);
    });

    // Act
    app.shutdown();
    app.join().await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn shutdown_aborts_root_task_after_cancel_timeout() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder()
        .cancel_timeout(Duration::from_millis(10))
        .insert_resource(Started::default())
        .insert_resource(Aborted::default())
        .build();

    app.spawn(async |started: Res<Started>, aborted: Res<Aborted>| {
        let _guard = AbortGuard(aborted);
        started.0.notify_one();
        std::future::pending::<()>().await;
    });

    app.get::<Started>()?.0.notified().await;

    // Act
    app.shutdown();
    app.join().await?;

    // Assert
    timeout(Duration::from_secs(1), app.get::<Aborted>()?.0.notified())
        .await
        .unwrap();

    Ok(())
}

#[tokio::test]
async fn defer_runs_after_task_requested_shutdown() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder().insert_resource(Flag::default()).build();

    app.defer(async |cleaned_up: Res<Flag>| {
        cleaned_up.0.store(true, Ordering::SeqCst);
    });
    app.spawn(|shutdown: Shutdown| async move {
        shutdown.now();
    });

    // Act
    app.join().await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn root_failure_error_wins_over_defer_error() {
    // Arrange
    let mut app = App::new();

    app.defer(async || -> Result<(), _> { Err("defer failed") });
    app.spawn(async || -> Result<(), _> { Err("root failed") });

    // Act
    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
}

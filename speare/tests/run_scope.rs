use eyre::eyre;
use speare::Error;
use speare::prelude::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct Flag(AtomicBool);

#[derive(Default)]
struct Notifier(Notify);

#[derive(Default)]
struct SeenError(Mutex<Option<String>>);

#[tokio::test]
async fn run_waits_for_scoped_children_before_returning() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.run(async |scope: Scope| {
        scope.spawn(async |child_ran: Res<Flag>| {
            child_ran.0.store(true, Ordering::SeqCst);
        });
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn run_parent_failure_cancels_scoped_children() {
    // Arrange
    let app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .insert_resource(Flag::default())
        .insert_resource(Notifier::default())
        .build();

    // Act
    let err = app
        .run(
            async |scope: Scope, child_started: Res<Notifier>| -> Result<(), _> {
                scope.spawn(
                    async |scope: Scope,
                           child_started: Res<Notifier>,
                           child_cancelled: Res<Flag>| {
                        child_started.0.notify_one();
                        scope.cancelled().await;
                        child_cancelled.0.store(true, Ordering::SeqCst);
                    },
                );

                child_started.0.notified().await;
                Err("run failed")
            },
        )
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "run failed");
    assert!(app.get::<Flag>().unwrap().0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn scope_spawn_ignores_successful_child_task_value() -> Result<(), Error> {
    // Arrange
    let app = App::new();

    // Act
    let result = app
        .run(async |scope: Scope| {
            scope.spawn(async || -> Result<_, Error> { Ok(42i32) });
        })
        .await;

    // Assert
    assert!(result.is_ok());
    Ok(())
}

#[tokio::test]
async fn run_executes_independently_of_root_tasks() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder()
        .insert_resource(Notifier::default())
        .insert_resource(Flag::default())
        .build();

    app.spawn(async |scope: Scope, root_started: Res<Notifier>| {
        root_started.0.notify_one();
        scope.cancelled().await;
    });

    app.get::<Notifier>()?.0.notified().await;

    // Act
    app.run(async |run_completed: Res<Flag>| {
        run_completed.0.store(true, Ordering::SeqCst);
    })
    .await?;

    app.shutdown();
    app.join().await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn scope_run_returns_task_value() -> Result<(), Error> {
    // Arrange
    let app = App::new();

    // Act
    let value = app
        .run(async |scope: Scope| -> Result<i32, Error> {
            Ok(scope.run(async || -> Result<i32, Error> { Ok(42) }).await?)
        })
        .await?;

    // Assert
    assert_eq!(value, 42);
    Ok(())
}

#[tokio::test]
async fn scope_run_preserves_task_error_type() -> Result<(), Error> {
    // Arrange
    let app = App::builder()
        .insert_resource(SeenError::default())
        .build();

    // Act
    app.run(async |scope: Scope, seen_error: Res<SeenError>| {
        let err = scope
            .run(async || -> eyre::Result<()> { Err(eyre!("nested failed")) })
            .await
            .unwrap_err();

        let mut seen_error = seen_error.0.lock().unwrap();
        *seen_error = Some(err.to_string());
    })
    .await?;

    // Assert
    assert_eq!(
        app.get::<SeenError>()?.0.lock().unwrap().as_deref(),
        Some("nested failed")
    );
    Ok(())
}

#[tokio::test]
async fn scope_run_waits_for_nested_children() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.run(async |scope: Scope| {
        let result = scope
            .run(async |scope: Scope| {
                scope.spawn(async |child_ran: Res<Flag>| {
                    child_ran.0.store(true, Ordering::SeqCst);
                });
            })
            .await;

        assert!(result.is_ok());
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn scope_run_failure_cancels_nested_children() -> Result<(), Error> {
    // Arrange
    let app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .insert_resource(Flag::default())
        .insert_resource(Notifier::default())
        .build();

    // Act
    app.run(async |scope: Scope| {
        let err = scope
            .run(async |scope: Scope, child_started: Res<Notifier>| -> Result<(), _> {
                scope.spawn(
                    async |scope: Scope,
                           child_started: Res<Notifier>,
                           child_cancelled: Res<Flag>| {
                        child_started.0.notify_one();
                        scope.cancelled().await;
                        child_cancelled.0.store(true, Ordering::SeqCst);
                    },
                );

                child_started.0.notified().await;
                Err("nested failed")
            })
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "nested failed");
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

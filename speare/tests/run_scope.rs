use speare::Error;
use speare::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct Flag(AtomicBool);

#[derive(Default)]
struct Notifier(Notify);

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

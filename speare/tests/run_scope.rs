use speare::prelude::*;
use speare::Error;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;

#[tokio::test]
async fn run_waits_for_scoped_children_before_returning() -> Result<(), Error> {
    // Arrange
    let child_ran = Arc::new(AtomicBool::new(false));
    let app = App::new();

    // Act
    app.run({
        let child_ran = child_ran.clone();
        move |scope: Scope| {
            let child_ran = child_ran.clone();

            async move {
                scope.spawn(move || {
                    let child_ran = child_ran.clone();

                    async move {
                        child_ran.store(true, Ordering::SeqCst);
                    }
                });
            }
        }
    })
    .await?;

    // Assert
    assert!(child_ran.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn run_parent_failure_cancels_scoped_children() {
    // Arrange
    let child_started = Arc::new(Notify::new());
    let child_cancelled = Arc::new(AtomicBool::new(false));
    let app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .build();

    // Act
    let err = app
        .run({
            let child_started = child_started.clone();
            let child_cancelled = child_cancelled.clone();

            move |scope: Scope| {
                let child_started = child_started.clone();
                let child_cancelled = child_cancelled.clone();

                async move {
                    let wait_until_child_starts = child_started.clone();

                    scope.spawn(move |scope: Scope| {
                        let child_started = child_started.clone();
                        let child_cancelled = child_cancelled.clone();

                        async move {
                            child_started.notify_one();
                            scope.cancelled().await;
                            child_cancelled.store(true, Ordering::SeqCst);
                        }
                    });

                    wait_until_child_starts.notified().await;
                    Err::<(), _>(Error::source(std::io::Error::other("run failed")))
                }
            }
        })
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "run failed");
    assert!(child_cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn scope_spawn_ignores_successful_child_task_value() -> Result<(), Error> {
    // Arrange
    let app = App::new();

    // Act
    let result = app
        .run(|scope: Scope| async move {
            scope.spawn(|| async { Ok::<_, Error>(42i32) });
        })
        .await;

    // Assert
    assert!(result.is_ok());
    Ok(())
}

#[tokio::test]
async fn run_executes_independently_of_root_tasks() -> Result<(), Error> {
    // Arrange
    let root_started = Arc::new(Notify::new());
    let run_completed = Arc::new(AtomicBool::new(false));
    let mut app = App::new();

    app.spawn({
        let root_started = root_started.clone();
        move |scope: Scope| {
            let root_started = root_started.clone();

            async move {
                root_started.notify_one();
                scope.cancelled().await;
            }
        }
    });

    root_started.notified().await;

    // Act
    app.run({
        let run_completed = run_completed.clone();
        move || {
            let run_completed = run_completed.clone();

            async move {
                run_completed.store(true, Ordering::SeqCst);
            }
        }
    })
    .await?;

    app.shutdown();
    app.join().await?;

    // Assert
    assert!(run_completed.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn run_extracts_scope_and_shutdown_without_resources() -> Result<(), Error> {
    // Arrange
    let child_cancelled = Arc::new(AtomicBool::new(false));
    let app = App::new();

    // Act
    app.run({
        let child_cancelled = child_cancelled.clone();

        move |scope: Scope, shutdown: Shutdown| {
            let child_cancelled = child_cancelled.clone();

            async move {
                scope.spawn(move |scope: Scope| {
                    let child_cancelled = child_cancelled.clone();

                    async move {
                        scope.cancelled().await;
                        child_cancelled.store(true, Ordering::SeqCst);
                    }
                });

                scope.cancel_all();
                shutdown.now();
            }
        }
    })
    .await?;

    // Assert
    assert!(child_cancelled.load(Ordering::SeqCst));
    Ok(())
}

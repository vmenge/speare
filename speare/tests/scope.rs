use speare::Error;
use speare::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
async fn spawned_child_runs_before_parent_completes() -> Result<(), Error> {
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
async fn child_observes_child_cancel() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.run(async |scope: Scope| {
        let child = scope.spawn(async |scope: Scope, child_cancelled: Res<Flag>| {
            scope.cancelled().await;
            child_cancelled.0.store(true, Ordering::SeqCst);
        });

        child.cancel();
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn child_observes_scope_cancel_all() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.run(async |scope: Scope| {
        scope.spawn(async |scope: Scope, child_cancelled: Res<Flag>| {
            scope.cancelled().await;
            child_cancelled.0.store(true, Ordering::SeqCst);
        });

        scope.cancel_all();
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn child_cancel_with_aborts_child_that_ignores_cancellation() -> Result<(), Error> {
    // Arrange
    let app = App::builder()
        .insert_resource(Started::default())
        .insert_resource(Aborted::default())
        .build();

    // Act
    app.run(async |scope: Scope, started: Res<Started>| {
        let child = scope.spawn(async |started: Res<Started>, aborted: Res<Aborted>| {
            let _guard = AbortGuard(aborted);
            started.0.notify_one();
            std::future::pending::<()>().await;
        });

        started.0.notified().await;
        child.cancel_with(Duration::from_millis(10)).await;
    })
    .await?;

    // Assert
    timeout(
        Duration::from_secs(1),
        app.get::<Aborted>().unwrap().0.notified(),
    )
    .await
    .unwrap();

    Ok(())
}

#[tokio::test]
async fn parent_failure_cancels_child_before_abort() {
    // Arrange
    let app = App::builder()
        .cancel_timeout(Duration::from_secs(1))
        .insert_resource(Flag::default())
        .build();

    // Act
    let err = app
        .run(async |scope: Scope| -> Result<(), _> {
            scope.spawn(async |scope: Scope, child_cancelled: Res<Flag>| {
                scope.cancelled().await;
                child_cancelled.0.store(true, Ordering::SeqCst);
            });

            Err("parent failed")
        })
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "parent failed");
    assert!(app.get::<Flag>().unwrap().0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancellation_cascades_to_nested_children() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.run(async |scope: Scope| {
        let child = scope.spawn(async |scope: Scope| {
            scope.spawn(async |scope: Scope, grandchild_cancelled: Res<Flag>| {
                scope.cancelled().await;
                grandchild_cancelled.0.store(true, Ordering::SeqCst);
            });

            scope.cancelled().await;
        });

        child.cancel();
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn child_failure_does_not_fail_parent_scope() -> Result<(), Error> {
    App::new()
        .run(async |scope: Scope| {
            scope.spawn(async || -> Result<(), _> { Err("child failed") });
        })
        .await?;

    Ok(())
}

#[tokio::test]
async fn child_handle_can_watch_its_failure() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.run(async |scope: Scope, child_failure_seen: Res<Flag>| {
        let child = scope.spawn(async || -> Result<(), _> { Err("child failed") });

        let error = child.failed().await;
        assert_eq!(error.to_string(), "child failed");

        child_failure_seen.0.store(true, Ordering::SeqCst);
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn scope_can_watch_next_child_failure() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Flag::default()).build();

    // Act
    app.run(async |scope: Scope, scope_failure_seen: Res<Flag>| {
        scope.spawn(async || -> Result<(), _> { Err("child failed") });

        let ChildFailure { error, .. } = scope.next_failure().await;
        assert_eq!(error.to_string(), "child failed");

        scope_failure_seen.0.store(true, Ordering::SeqCst);
    })
    .await?;

    // Assert
    assert!(app.get::<Flag>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn parent_can_explicitly_fail_from_child_failure() {
    let err = App::new()
        .run(async |scope: Scope| -> Result<(), _> {
            scope.spawn(async || -> Result<(), _> { Err("child failed") });

            let failure = scope.next_failure().await;
            Err(failure.error)
        })
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "child failed");
}

#[tokio::test]
async fn parent_failure_aborts_spawned_children() {
    // Arrange
    let app = App::builder()
        .insert_resource(Started::default())
        .insert_resource(Aborted::default())
        .build();

    // Act
    let err = app
        .run(
            async |scope: Scope, child_started: Res<Started>| -> Result<(), _> {
                let child = scope.spawn(
                    async |child_aborted: Res<Aborted>, child_started: Res<Started>| {
                        let _guard = AbortGuard(child_aborted);
                        child_started.0.notify_one();

                        std::future::pending::<()>().await;
                    },
                );

                child_started.0.notified().await;
                child.abort();

                Err("parent failed")
            },
        )
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "parent failed");
    timeout(
        Duration::from_secs(1),
        app.get::<Aborted>().unwrap().0.notified(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn retry_respawns_children() -> Result<(), Error> {
    #[derive(Default)]
    struct Attempts(AtomicUsize);

    #[derive(Default)]
    struct ChildRuns(AtomicUsize);

    // Arrange
    let app = App::builder()
        .insert_resource(Attempts::default())
        .insert_resource(ChildRuns::default())
        .build();

    // Act
    let task = async |scope: Scope, attempts: Res<Attempts>| {
        scope.spawn(async |child_runs: Res<ChildRuns>| {
            child_runs.0.fetch_add(1, Ordering::SeqCst);
        });

        let attempt = attempts.0.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            return Err("retry");
        }

        Ok(())
    };

    app.run(task.retry(Retry::Max(1))).await?;

    // Assert
    assert_eq!(app.get::<Attempts>()?.0.load(Ordering::SeqCst), 2);
    assert_eq!(app.get::<ChildRuns>()?.0.load(Ordering::SeqCst), 2);
    Ok(())
}

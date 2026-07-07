use speare::Error;
use speare::prelude::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Events(Mutex<Vec<&'static str>>);

#[derive(Default)]
struct Attempts(AtomicUsize);

#[tokio::test]
async fn scope_defer_runs_after_successful_task() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Events::default()).build();

    // Act
    app.run(async |scope: Scope, events: Res<Events>| {
        scope.defer(async |events: Res<Events>| {
            events.0.lock().unwrap().push("defer");
        });

        events.0.lock().unwrap().push("task");
    })
    .await?;

    // Assert
    assert_eq!(
        *app.get::<Events>()?.0.lock().unwrap(),
        ["task", "defer"]
    );
    Ok(())
}

#[tokio::test]
async fn scope_defer_runs_after_successful_spawned_task() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder().insert_resource(Events::default()).build();

    // Act
    app.spawn(async |scope: Scope, events: Res<Events>| {
        scope.defer(async |events: Res<Events>| {
            events.0.lock().unwrap().push("defer");
        });

        events.0.lock().unwrap().push("task");
    })
    .join()
    .await?;

    // Assert
    assert_eq!(
        *app.get::<Events>()?.0.lock().unwrap(),
        ["task", "defer"]
    );
    Ok(())
}

#[tokio::test]
async fn scope_defer_error_surfaces_from_successful_spawned_task() {
    // Arrange
    let mut app = App::new();

    // Act
    let err = app
        .spawn(async |scope: Scope| {
            scope.defer(async || -> Result<(), _> { Err("defer failed") });
        })
        .join()
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "defer failed");
}

#[tokio::test]
async fn spawned_task_failure_takes_precedence_over_scope_defer_failure() {
    // Arrange
    let mut app = App::new();

    // Act
    let err = app
        .spawn(async |scope: Scope| -> Result<(), _> {
            scope.defer(async || -> Result<(), _> { Err("defer failed") });
            Err("task failed")
        })
        .join()
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "task failed");
}

#[tokio::test]
async fn scope_defer_runs_after_spawned_task_failure() {
    // Arrange
    let mut app = App::builder().insert_resource(Events::default()).build();

    // Act
    let err = app
        .spawn(async |scope: Scope| -> Result<(), _> {
            scope.defer(async |events: Res<Events>| {
                events.0.lock().unwrap().push("defer");
            });

            Err("task failed")
        })
        .join()
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "task failed");
    assert_eq!(*app.get::<Events>().unwrap().0.lock().unwrap(), ["defer"]);
}

#[tokio::test]
async fn scope_defer_error_surfaces_when_task_succeeds() {
    // Arrange
    let app = App::new();

    // Act
    let err = app
        .run(async |scope: Scope| {
            scope.defer(async || -> Result<(), _> { Err("defer failed") });
        })
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "defer failed");
}

#[tokio::test]
async fn scope_task_failure_takes_precedence_over_defer_failure() {
    // Arrange
    let app = App::new();

    // Act
    let err = app
        .run(async |scope: Scope| -> Result<(), _> {
            scope.defer(async || -> Result<(), _> { Err("defer failed") });
            Err("task failed")
        })
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "task failed");
}

#[tokio::test]
async fn scope_defer_runs_after_task_failure() {
    // Arrange
    let app = App::builder().insert_resource(Events::default()).build();

    // Act
    let err = app
        .run(async |scope: Scope| -> Result<(), _> {
            scope.defer(async |events: Res<Events>| {
                events.0.lock().unwrap().push("defer");
            });

            Err("task failed")
        })
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "task failed");
    assert_eq!(*app.get::<Events>().unwrap().0.lock().unwrap(), ["defer"]);
}

#[tokio::test]
async fn scope_defer_runs_after_children_settle() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Events::default()).build();

    // Act
    app.run(async |scope: Scope| {
        scope.defer(async |events: Res<Events>| {
            events.0.lock().unwrap().push("defer");
        });

        scope.spawn(async |events: Res<Events>| {
            events.0.lock().unwrap().push("child");
        });
    })
    .await?;

    // Assert
    assert_eq!(*app.get::<Events>()?.0.lock().unwrap(), ["child", "defer"]);
    Ok(())
}

#[tokio::test]
async fn scope_defer_runs_once_per_retry_attempt() -> Result<(), Error> {
    // Arrange
    let app = App::builder()
        .insert_resource(Attempts::default())
        .insert_resource(Events::default())
        .build();

    let task = async |scope: Scope, attempts: Res<Attempts>, events: Res<Events>| -> Result<(), _> {
        events.0.lock().unwrap().push("task");
        scope.defer(async |events: Res<Events>| {
            events.0.lock().unwrap().push("defer");
        });

        let attempt = attempts.0.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            return Err("retry");
        }

        Ok(())
    };

    // Act
    app.run(task.retry(Retry::Max(1))).await?;

    // Assert
    assert_eq!(
        *app.get::<Events>()?.0.lock().unwrap(),
        ["task", "defer", "task", "defer"]
    );
    Ok(())
}

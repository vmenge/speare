use speare::Error;
use speare::prelude::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct RootCompleted(AtomicBool);

#[derive(Default)]
struct CleanedUp(AtomicBool);

#[derive(Default)]
struct Events(Mutex<Vec<&'static str>>);

#[tokio::test]
async fn defer_runs_after_successful_root_completion() -> Result<(), Error> {
    // Arrange

    let mut app = App::builder()
        .insert_resource(RootCompleted::default())
        .insert_resource(CleanedUp::default())
        .build();

    // Act
    app.defer(async |rc: Res<RootCompleted>, cu: Res<CleanedUp>| {
        if rc.0.load(Ordering::SeqCst) {
            cu.0.store(true, Ordering::SeqCst);
        }
    })
    .spawn(async |rc: Res<RootCompleted>| {
        rc.0.store(true, Ordering::SeqCst);
    })
    .join()
    .await?;

    // Assert
    assert!(app.get::<CleanedUp>()?.0.load(Ordering::SeqCst));

    Ok(())
}

#[tokio::test]
async fn defer_supports_heterogeneous_cleanup_callbacks() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder().insert_resource(Events::default()).build();

    // Act
    app.defer(async |events: Res<Events>| {
        events.0.lock().unwrap().push("first");
    })
    .defer(async |events: Res<Events>| {
        events.0.lock().unwrap().push("second");
        Ok::<_, Error>(())
    })
    .join()
    .await?;

    // Assert
    assert_eq!(*app.get::<Events>()?.0.lock().unwrap(), ["first", "second"]);

    Ok(())
}

#[tokio::test]
async fn defer_runs_after_root_failure() {
    // Arrange
    let mut app = App::builder().insert_resource(CleanedUp::default()).build();

    // Act
    let err = app
        .defer(async |cleaned_up: Res<CleanedUp>| {
            cleaned_up.0.store(true, Ordering::SeqCst);
        })
        .spawn(async || -> Result<(), _> { Err("root failed") })
        .join()
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
    assert!(app.get::<CleanedUp>().unwrap().0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn defer_runs_after_shutdown() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder().insert_resource(CleanedUp::default()).build();

    app.defer(async |cleaned_up: Res<CleanedUp>| {
        cleaned_up.0.store(true, Ordering::SeqCst);
    })
    .spawn(|scope: Scope| async move {
        scope.cancelled().await;
    });

    // Act
    app.shutdown();
    app.join().await?;

    // Assert
    assert!(app.get::<CleanedUp>()?.0.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn defer_error_surfaces_when_root_tasks_succeed() {
    // Arrange
    let mut app = App::new();

    // Act
    let err = app
        .defer(async || -> Result<(), _> { Err("defer failed") })
        .spawn(|| async {})
        .join()
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "defer failed");
}

#[tokio::test]
async fn root_failure_takes_precedence_over_defer_failure() {
    // Arrange
    let mut app = App::new();

    // Act
    let err = app
        .defer(async || -> Result<(), _> { Err("defer failed") })
        .spawn(async || -> Result<(), _> { Err("root failed") })
        .join()
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
}

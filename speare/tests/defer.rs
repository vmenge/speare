use speare::Error;
use speare::prelude::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[tokio::test]
async fn defer_runs_after_successful_root_completion() -> Result<(), Error> {
    // Arrange
    let root_completed = Arc::new(AtomicBool::new(false));
    let cleaned_up = Arc::new(AtomicBool::new(false));
    let mut app = App::new();

    app.defer({
        let root_completed = root_completed.clone();
        let cleaned_up = cleaned_up.clone();

        move || {
            let root_completed = root_completed.clone();
            let cleaned_up = cleaned_up.clone();

            async move {
                if root_completed.load(Ordering::SeqCst) {
                    cleaned_up.store(true, Ordering::SeqCst);
                }
            }
        }
    });

    // Act
    app.spawn({
        let root_completed = root_completed.clone();

        move || {
            let root_completed = root_completed.clone();

            async move {
                root_completed.store(true, Ordering::SeqCst);
            }
        }
    });
    app.join().await?;

    // Assert
    assert!(cleaned_up.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn defer_supports_heterogeneous_cleanup_callbacks() -> Result<(), Error> {
    // Arrange
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = App::new();

    app.defer({
        let events = events.clone();

        move || {
            let events = events.clone();

            async move {
                events.lock().unwrap().push("first");
            }
        }
    });

    app.defer({
        let events = events.clone();

        move || {
            let events = events.clone();

            async move {
                events.lock().unwrap().push("second");
                Ok::<_, Error>(())
            }
        }
    });

    // Act
    app.spawn(|| async {});
    app.join().await?;

    // Assert
    assert_eq!(*events.lock().unwrap(), ["first", "second"]);
    Ok(())
}

#[tokio::test]
async fn defer_runs_after_root_failure() {
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

    // Act
    app.spawn(|| async { Err::<(), _>(Error::source(std::io::Error::other("root failed"))) });
    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
    assert!(cleaned_up.load(Ordering::SeqCst));
}

#[tokio::test]
async fn defer_runs_after_shutdown() -> Result<(), Error> {
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

    app.spawn(|scope: Scope| async move {
        scope.cancelled().await;
    });

    // Act
    app.shutdown();
    app.join().await?;

    // Assert
    assert!(cleaned_up.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn defer_error_surfaces_when_root_tasks_succeed() {
    // Arrange
    let mut app = App::new();

    app.defer(|| async { Err::<(), _>(Error::source(std::io::Error::other("defer failed"))) });

    // Act
    app.spawn(|| async {});
    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "defer failed");
}

#[tokio::test]
async fn root_failure_takes_precedence_over_defer_failure() {
    // Arrange
    let mut app = App::new();

    app.defer(|| async { Err::<(), _>(Error::source(std::io::Error::other("defer failed"))) });

    // Act
    app.spawn(|| async { Err::<(), _>(Error::source(std::io::Error::other("root failed"))) });
    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "root failed");
}

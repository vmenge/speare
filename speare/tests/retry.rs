use speare::Error;
use speare::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Attempts(AtomicUsize);

#[test]
fn retry_defaults_to_never() {
    assert_eq!(Retry::default(), Retry::Never);
}

#[test]
fn backoff_defaults_to_none() {
    assert_eq!(Backoff::default(), Backoff::None);
}

#[tokio::test]
async fn never_retries_failed_task() {
    // Arrange
    let app = App::builder().insert_resource(Attempts::default()).build();
    let inc_attempts = async |attempts: Res<Attempts>| -> Result<(), _> {
        attempts.0.fetch_add(1, Ordering::SeqCst);

        Err(Error::Report {
            message: "failed".into(),
        })
    };

    // Act
    let err = app.run(inc_attempts.retry(Retry::Never)).await.unwrap_err();

    // Assert
    assert_eq!(app.get::<Attempts>().unwrap().0.load(Ordering::SeqCst), 1);
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn max_retries_failed_task() {
    // Arrange
    let app = App::builder().insert_resource(Attempts::default()).build();
    let inc_attempts = async |attempts: Res<Attempts>| -> Result<(), _> {
        attempts.0.fetch_add(1, Ordering::SeqCst);

        Err(Error::Report {
            message: "failed".into(),
        })
    };

    // Act
    let err = App::new()
        .run(inc_attempts.retry(Retry::Max(2)))
        .await
        .unwrap_err();

    // Assert
    assert_eq!(app.get::<Attempts>().unwrap().0.load(Ordering::SeqCst), 3);
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn always_retries_until_success() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(Attempts::default()).build();
    let inc_attempts = async |attempts: Res<Attempts>| -> Result<(), _> {
        let attempt = attempts.0.fetch_add(1, Ordering::SeqCst);

        if attempt < 2 {
            return Err(Error::Report {
                message: "failed".into(),
            });
        }

        Ok(())
    };

    // Act
    App::new().run(inc_attempts.retry(Retry::Always)).await?;

    // Assert
    assert_eq!(app.get::<Attempts>().unwrap().0.load(Ordering::SeqCst), 3);
    Ok(())
}

use speare::prelude::*;
use speare::{Error, RunError};
use std::error;
use std::fmt;

#[derive(Debug)]
struct ReportLikeError(&'static str);

impl fmt::Display for ReportLikeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Debug)]
struct OriginalRunError;

impl fmt::Display for OriginalRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("original run error")
    }
}

impl error::Error for OriginalRunError {}

#[tokio::test]
async fn run_returns_successful_task_value() -> Result<(), Error> {
    // Arrange
    let app = App::new();

    // Act
    let value = app.run(async || -> Result<i32, Error> { Ok(42) }).await?;

    // Assert
    assert_eq!(value, 42);
    Ok(())
}

#[tokio::test]
async fn run_returns_original_task_error_type() {
    // Arrange
    let app = App::new();

    // Act
    let err = app
        .run(async || -> Result<(), OriginalRunError> { Err(OriginalRunError) })
        .await
        .unwrap_err();

    // Assert
    match &err {
        RunError::Task(error) => assert_eq!(error.to_string(), "original run error"),
        RunError::App(error) => panic!("expected task error, got app error: {error}"),
    }

    assert_eq!(err.to_string(), "original run error");
    assert_eq!(
        error::Error::source(&err).unwrap().to_string(),
        "original run error"
    );
}

#[tokio::test]
async fn run_still_accepts_unit_task_output() -> Result<(), Error> {
    assert_eq!((), App::new().run(async || {}).await?);
    Ok(())
}

#[tokio::test]
async fn run_maps_extraction_failure_to_app_error() {
    // Arrange / Act
    let err = App::new().run(async |_: Res<i32>| {}).await.unwrap_err();

    // Assert
    match err {
        RunError::Task(error) => match error {},
        RunError::App(error) => assert_eq!(error.to_string(), "i32 is not registered"),
    }
}

#[tokio::test]
async fn spawn_erases_task_error() {
    // Arrange
    let mut app = App::new();

    // Act
    app.spawn(async || -> Result<(), ReportLikeError> { Err(ReportLikeError("spawn failed")) });
    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "spawn failed");
    assert!(matches!(
        err,
        Error::Report { message } if message == "spawn failed"
    ));
}

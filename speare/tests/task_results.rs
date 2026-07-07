use speare::prelude::*;
use std::fmt;

#[derive(Debug)]
struct ReportLikeError(&'static str);

impl fmt::Display for ReportLikeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[tokio::test]
async fn run_returns_successful_task_value() -> Result<()> {
    // Arrange
    let app = App::new();

    // Act
    let value = app.run(|| async { Ok::<i32, Error>(42) }).await?;

    // Assert
    assert_eq!(value, 42);
    Ok(())
}

#[tokio::test]
async fn run_still_accepts_unit_task_output() -> Result<()> {
    // Arrange
    let app = App::new();

    // Act
    let value = app.run(|| async {}).await?;

    // Assert
    assert_eq!(value, ());
    Ok(())
}

#[tokio::test]
async fn run_erases_std_error_from_result_task_output() {
    // Arrange
    let app = App::new();

    // Act
    let err = app
        .run(|| async { Err::<i32, _>(std::io::Error::other("io failed")) })
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "io failed");
}

#[tokio::test]
async fn run_erases_report_like_error_without_std_error_bound() {
    // Arrange
    let app = App::new();

    // Act
    let err = app
        .run(|| async { Err::<i32, _>(ReportLikeError("report failed")) })
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "report failed");
}

use speare::{App, Backoff, Error, Result, Retry, Schedule, TaskExt};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

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
    let attempts = Arc::new(AtomicUsize::new(0));
    let task_attempts = attempts.clone();

    let err = App::new()
        .add_task(
            Schedule::Start,
            (move || {
                let task_attempts = task_attempts.clone();

                async move {
                    task_attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(Error::source(std::io::Error::other("failed")))
                }
            })
            .retry(Retry::Never),
        )
        .start()
        .await
        .unwrap_err();

    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn max_retries_failed_task() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let task_attempts = attempts.clone();

    let err = App::new()
        .add_task(
            Schedule::Start,
            (move || {
                let task_attempts = task_attempts.clone();

                async move {
                    task_attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(Error::source(std::io::Error::other("failed")))
                }
            })
            .retry(Retry::Max(2)),
        )
        .start()
        .await
        .unwrap_err();

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn always_retries_until_success() -> Result<()> {
    let attempts = Arc::new(AtomicUsize::new(0));
    let task_attempts = attempts.clone();

    App::new()
        .add_task(
            Schedule::Start,
            (move || {
                let task_attempts = task_attempts.clone();

                async move {
                    let attempt = task_attempts.fetch_add(1, Ordering::SeqCst);

                    if attempt < 2 {
                        Err::<(), _>(Error::source(std::io::Error::other("failed")))
                    } else {
                        Ok(())
                    }
                }
            })
            .retry(Retry::Always),
        )
        .start()
        .await?;

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    Ok(())
}

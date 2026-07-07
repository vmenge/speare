use speare::{App, Backoff, Error, Result, Retry, Schedule, SystemExt};
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
async fn never_retries_failed_system() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let system_attempts = attempts.clone();

    let err = App::new()
        .add_system(
            Schedule::Start,
            (move || {
                let system_attempts = system_attempts.clone();

                async move {
                    system_attempts.fetch_add(1, Ordering::SeqCst);
                    Err(Error::source(std::io::Error::other("failed")))
                }
            })
            .retry(Retry::Never),
        )
        .spawn()
        .join()
        .await
        .unwrap_err();

    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn max_retries_failed_system() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let system_attempts = attempts.clone();

    let err = App::new()
        .add_system(
            Schedule::Start,
            (move || {
                let system_attempts = system_attempts.clone();

                async move {
                    system_attempts.fetch_add(1, Ordering::SeqCst);
                    Err(Error::source(std::io::Error::other("failed")))
                }
            })
            .retry(Retry::Max(2)),
        )
        .spawn()
        .join()
        .await
        .unwrap_err();

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn always_retries_until_success() -> Result<()> {
    let attempts = Arc::new(AtomicUsize::new(0));
    let system_attempts = attempts.clone();

    App::new()
        .add_system(
            Schedule::Start,
            (move || {
                let system_attempts = system_attempts.clone();

                async move {
                    let attempt = system_attempts.fetch_add(1, Ordering::SeqCst);

                    if attempt < 2 {
                        Err(Error::source(std::io::Error::other("failed")))
                    } else {
                        Ok(())
                    }
                }
            })
            .retry(Retry::Always),
        )
        .spawn()
        .join()
        .await?;

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    Ok(())
}

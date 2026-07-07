use speare::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{
    sync::Notify,
    time::{Duration, timeout},
};

#[tokio::test]
async fn normal_completion_runs_shutdown() -> Result<()> {
    let shutdown_ran = Arc::new(AtomicBool::new(false));

    App::new()
        .add_task(Schedule::Work, || async {})
        .add_task(Schedule::Shutdown, {
            let shutdown_ran = shutdown_ran.clone();

            move || {
                let shutdown_ran = shutdown_ran.clone();

                async move {
                    shutdown_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await?;

    assert!(shutdown_ran.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn task_can_request_shutdown() -> Result<()> {
    let shutdown_ran = Arc::new(AtomicBool::new(false));

    App::new()
        .add_task(Schedule::Work, |shutdown: Shutdown| async move {
            shutdown.now();
        })
        .add_task(Schedule::Shutdown, {
            let shutdown_ran = shutdown_ran.clone();

            move || {
                let shutdown_ran = shutdown_ran.clone();

                async move {
                    shutdown_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await?;

    assert!(shutdown_ran.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn shutdown_cancels_running_tasks() -> Result<()> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let started = Arc::new(Notify::new());

    App::new()
        .cancel_timeout(Duration::from_secs(1))
        .add_task(Schedule::Work, {
            let started = started.clone();

            move |shutdown: Shutdown| {
                let started = started.clone();

                async move {
                    started.notified().await;
                    shutdown.now();
                }
            }
        })
        .add_task(Schedule::Work, {
            let cancelled = cancelled.clone();
            let started = started.clone();

            move |scope: Scope| {
                let cancelled = cancelled.clone();
                let started = started.clone();

                async move {
                    started.notify_one();
                    scope.cancelled().await;
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await?;

    assert!(cancelled.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn shutdown_aborts_task_after_cancel_timeout() -> Result<()> {
    let started = Arc::new(Notify::new());
    let aborted = Arc::new(Notify::new());
    let task_started = started.clone();
    let task_aborted = aborted.clone();

    App::new()
        .cancel_timeout(Duration::from_millis(10))
        .add_task(Schedule::Work, move || {
            let started = task_started.clone();
            let aborted = task_aborted.clone();

            async move {
                struct AbortGuard(Arc<Notify>);

                impl Drop for AbortGuard {
                    fn drop(&mut self) {
                        self.0.notify_one();
                    }
                }

                let _guard = AbortGuard(aborted);
                started.notify_one();
                std::future::pending::<()>().await;
            }
        })
        .add_task(Schedule::Work, {
            let started = started.clone();

            move |shutdown: Shutdown| {
                let started = started.clone();

                async move {
                    started.notified().await;
                    shutdown.now();
                }
            }
        })
        .start()
        .await?;

    timeout(Duration::from_secs(1), aborted.notified())
        .await
        .unwrap();

    Ok(())
}

#[tokio::test]
async fn all_shutdown_tasks_run() -> Result<()> {
    let first = Arc::new(AtomicBool::new(false));
    let second = Arc::new(AtomicBool::new(false));
    let third = Arc::new(AtomicBool::new(false));

    App::new()
        .add_task(Schedule::Shutdown, {
            let first = first.clone();

            move || {
                let first = first.clone();

                async move {
                    first.store(true, Ordering::SeqCst);
                }
            }
        })
        .add_task(Schedule::Shutdown, {
            let second = second.clone();

            move || {
                let second = second.clone();

                async move {
                    second.store(true, Ordering::SeqCst);
                }
            }
        })
        .add_task(Schedule::Shutdown, {
            let third = third.clone();

            move || {
                let third = third.clone();

                async move {
                    third.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await?;

    assert!(first.load(Ordering::SeqCst));
    assert!(second.load(Ordering::SeqCst));
    assert!(third.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn all_shutdown_tasks_run_even_if_one_fails() {
    let later_ran = Arc::new(AtomicBool::new(false));

    let err = App::new()
        .add_task(Schedule::Shutdown, || async {
            Err(Error::source(std::io::Error::other("cleanup failed")))
        })
        .add_task(Schedule::Shutdown, {
            let later_ran = later_ran.clone();

            move || {
                let later_ran = later_ran.clone();

                async move {
                    later_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "cleanup failed");
    assert!(later_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn start_failure_still_runs_shutdown() {
    let shutdown_ran = Arc::new(AtomicBool::new(false));

    let err = App::new()
        .add_task(Schedule::Start, || async {
            Err(Error::source(std::io::Error::other("start failed")))
        })
        .add_task(Schedule::Shutdown, {
            let shutdown_ran = shutdown_ran.clone();

            move || {
                let shutdown_ran = shutdown_ran.clone();

                async move {
                    shutdown_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "start failed");
    assert!(shutdown_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn start_failure_error_wins_over_shutdown_error() {
    let err = App::new()
        .add_task(Schedule::Start, || async {
            Err(Error::source(std::io::Error::other("start failed")))
        })
        .add_task(Schedule::Shutdown, || async {
            Err(Error::source(std::io::Error::other("shutdown failed")))
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "start failed");
}

#[tokio::test]
async fn task_failure_still_runs_shutdown() {
    let shutdown_ran = Arc::new(AtomicBool::new(false));

    let err = App::new()
        .add_task(Schedule::Work, || async {
            Err(Error::source(std::io::Error::other("task failed")))
        })
        .add_task(Schedule::Shutdown, {
            let shutdown_ran = shutdown_ran.clone();

            move || {
                let shutdown_ran = shutdown_ran.clone();

                async move {
                    shutdown_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "task failed");
    assert!(shutdown_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn task_failure_error_wins_over_shutdown_error() {
    let err = App::new()
        .add_task(Schedule::Work, || async {
            Err(Error::source(std::io::Error::other("task failed")))
        })
        .add_task(Schedule::Shutdown, || async {
            Err(Error::source(std::io::Error::other("shutdown failed")))
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "task failed");
}

#[tokio::test]
async fn shutdown_requested_during_start_skips_remaining_start_and_task() -> Result<()> {
    let later_start_ran = Arc::new(AtomicBool::new(false));
    let task_ran = Arc::new(AtomicBool::new(false));
    let shutdown_ran = Arc::new(AtomicBool::new(false));

    App::new()
        .add_task(Schedule::Start, |shutdown: Shutdown| async move {
            shutdown.now();
        })
        .add_task(Schedule::Start, {
            let later_start_ran = later_start_ran.clone();

            move || {
                let later_start_ran = later_start_ran.clone();

                async move {
                    later_start_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .add_task(Schedule::Work, {
            let task_ran = task_ran.clone();

            move || {
                let task_ran = task_ran.clone();

                async move {
                    task_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .add_task(Schedule::Shutdown, {
            let shutdown_ran = shutdown_ran.clone();

            move || {
                let shutdown_ran = shutdown_ran.clone();

                async move {
                    shutdown_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await?;

    assert!(!later_start_ran.load(Ordering::SeqCst));
    assert!(!task_ran.load(Ordering::SeqCst));
    assert!(shutdown_ran.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn shutdown_during_shutdown_is_no_op() -> Result<()> {
    let later_shutdown_ran = Arc::new(AtomicBool::new(false));

    App::new()
        .add_task(Schedule::Shutdown, |shutdown: Shutdown| async move {
            shutdown.now();
        })
        .add_task(Schedule::Shutdown, {
            let later_shutdown_ran = later_shutdown_ran.clone();

            move || {
                let later_shutdown_ran = later_shutdown_ran.clone();

                async move {
                    later_shutdown_ran.store(true, Ordering::SeqCst);
                }
            }
        })
        .start()
        .await?;

    assert!(later_shutdown_ran.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn shutdown_tasks_are_not_scope_cancelled_by_app_shutdown() -> Result<()> {
    App::new()
        .add_task(Schedule::Work, |shutdown: Shutdown| async move {
            shutdown.now();
        })
        .add_task(Schedule::Shutdown, |scope: Scope| async move {
            assert!(!scope.is_cancelled());
        })
        .start()
        .await
}

#[tokio::test]
async fn shutdown_tasks_respect_retry() -> Result<()> {
    let attempts = Arc::new(AtomicUsize::new(0));
    let task_attempts = attempts.clone();

    App::new()
        .add_task(
            Schedule::Shutdown,
            (move || {
                let task_attempts = task_attempts.clone();

                async move {
                    let attempt = task_attempts.fetch_add(1, Ordering::SeqCst);

                    if attempt == 0 {
                        Err(Error::source(std::io::Error::other("retry shutdown")))
                    } else {
                        Ok(())
                    }
                }
            })
            .retry(Retry::Max(1)),
        )
        .start()
        .await?;

    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    Ok(())
}

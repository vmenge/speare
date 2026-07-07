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
async fn spawned_child_runs_before_parent_completes() -> Result<()> {
    let ran = Arc::new(AtomicBool::new(false));
    let child_ran = ran.clone();

    App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let child_ran = child_ran.clone();

            async move {
                scope.spawn(move || {
                    let child_ran = child_ran.clone();

                    async move {
                        child_ran.store(true, Ordering::SeqCst);
                    }
                });
            }
        })
        .start()
        .await?;

    assert!(ran.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn child_observes_child_cancel() -> Result<()> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let child_cancelled = cancelled.clone();

    App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let child_cancelled = child_cancelled.clone();

            async move {
                let child = scope.spawn(move |scope: Scope| {
                    let child_cancelled = child_cancelled.clone();

                    async move {
                        scope.cancelled().await;
                        child_cancelled.store(true, Ordering::SeqCst);
                    }
                });

                child.cancel();
            }
        })
        .start()
        .await?;

    assert!(cancelled.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn child_observes_scope_cancel_all() -> Result<()> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let child_cancelled = cancelled.clone();

    App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let child_cancelled = child_cancelled.clone();

            async move {
                scope.spawn(move |scope: Scope| {
                    let child_cancelled = child_cancelled.clone();

                    async move {
                        scope.cancelled().await;
                        child_cancelled.store(true, Ordering::SeqCst);
                    }
                });

                scope.cancel_all();
            }
        })
        .start()
        .await?;

    assert!(cancelled.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn child_cancel_with_aborts_child_that_ignores_cancellation() -> Result<()> {
    let started = Arc::new(Notify::new());
    let aborted = Arc::new(Notify::new());
    let task_aborted = aborted.clone();

    App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let started = started.clone();
            let aborted = task_aborted.clone();

            async move {
                let wait_started = started.clone();

                let child = scope.spawn(move || {
                    let started = started.clone();
                    let aborted = aborted.clone();

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
                });

                wait_started.notified().await;
                child.cancel_with(Duration::from_millis(10)).await;
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
async fn parent_failure_cancels_child_before_abort() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let child_cancelled = cancelled.clone();

    let err = App::new()
        .cancel_timeout(Duration::from_secs(1))
        .add_task(Schedule::Start, move |scope: Scope| {
            let child_cancelled = child_cancelled.clone();

            async move {
                scope.spawn(move |scope: Scope| {
                    let child_cancelled = child_cancelled.clone();

                    async move {
                        scope.cancelled().await;
                        child_cancelled.store(true, Ordering::SeqCst);
                    }
                });

                Err(Error::source(std::io::Error::other("parent failed")))
            }
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "parent failed");
    assert!(cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancellation_cascades_to_nested_children() -> Result<()> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let grandchild_cancelled = cancelled.clone();

    App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let grandchild_cancelled = grandchild_cancelled.clone();

            async move {
                let child = scope.spawn(move |scope: Scope| {
                    let grandchild_cancelled = grandchild_cancelled.clone();

                    async move {
                        scope.spawn(move |scope: Scope| {
                            let grandchild_cancelled = grandchild_cancelled.clone();

                            async move {
                                scope.cancelled().await;
                                grandchild_cancelled.store(true, Ordering::SeqCst);
                            }
                        });

                        scope.cancelled().await;
                    }
                });

                child.cancel();
            }
        })
        .start()
        .await?;

    assert!(cancelled.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn child_failure_does_not_fail_parent_scope() -> Result<()> {
    App::new()
        .add_task(Schedule::Start, |scope: Scope| async move {
            scope.spawn(|| async { Err(Error::source(std::io::Error::other("child failed"))) });
        })
        .start()
        .await?;

    Ok(())
}

#[tokio::test]
async fn child_handle_can_watch_its_failure() -> Result<()> {
    let failure_seen = Arc::new(AtomicBool::new(false));
    let child_failure_seen = failure_seen.clone();

    App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let child_failure_seen = child_failure_seen.clone();

            async move {
                let child = scope
                    .spawn(|| async { Err(Error::source(std::io::Error::other("child failed"))) });

                let error = child.failed().await;
                assert_eq!(error.to_string(), "child failed");
                child_failure_seen.store(true, Ordering::SeqCst);
            }
        })
        .start()
        .await?;

    assert!(failure_seen.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn scope_can_watch_next_child_failure() -> Result<()> {
    let failure_seen = Arc::new(AtomicBool::new(false));
    let scope_failure_seen = failure_seen.clone();

    App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let scope_failure_seen = scope_failure_seen.clone();

            async move {
                scope.spawn(|| async { Err(Error::source(std::io::Error::other("child failed"))) });

                let ChildFailure { error, .. } = scope.next_failure().await;
                assert_eq!(error.to_string(), "child failed");
                scope_failure_seen.store(true, Ordering::SeqCst);
            }
        })
        .start()
        .await?;

    assert!(failure_seen.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn parent_can_explicitly_fail_from_child_failure() {
    let err = App::new()
        .add_task(Schedule::Start, |scope: Scope| async move {
            scope.spawn(|| async { Err(Error::source(std::io::Error::other("child failed"))) });

            let failure = scope.next_failure().await;
            Err(failure.error)
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "child failed");
}

#[tokio::test]
async fn parent_failure_aborts_spawned_children() {
    let started = Arc::new(Notify::new());
    let aborted = Arc::new(Notify::new());
    let child_started = started.clone();
    let child_aborted = aborted.clone();

    let err = App::new()
        .add_task(Schedule::Start, move |scope: Scope| {
            let child_started = child_started.clone();
            let child_aborted = child_aborted.clone();

            async move {
                let wait_started = child_started.clone();

                let child = scope.spawn(move || {
                    let child_started = child_started.clone();
                    let child_aborted = child_aborted.clone();

                    async move {
                        struct AbortGuard(Arc<Notify>);

                        impl Drop for AbortGuard {
                            fn drop(&mut self) {
                                self.0.notify_one();
                            }
                        }

                        let _guard = AbortGuard(child_aborted);
                        child_started.notify_one();
                        std::future::pending::<()>().await;
                    }
                });

                wait_started.notified().await;
                child.abort();
                Err(Error::source(std::io::Error::other("parent failed")))
            }
        })
        .start()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "parent failed");
    timeout(Duration::from_secs(1), aborted.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn retry_respawns_children() -> Result<()> {
    let attempts = Arc::new(AtomicUsize::new(0));
    let child_runs = Arc::new(AtomicUsize::new(0));

    let task_attempts = attempts.clone();
    let task_child_runs = child_runs.clone();

    App::new()
        .add_task(
            Schedule::Start,
            (move |scope: Scope| {
                let task_attempts = task_attempts.clone();
                let task_child_runs = task_child_runs.clone();

                async move {
                    scope.spawn(move || {
                        let task_child_runs = task_child_runs.clone();

                        async move {
                            task_child_runs.fetch_add(1, Ordering::SeqCst);
                        }
                    });

                    let attempt = task_attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        Err(Error::source(std::io::Error::other("retry")))
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
    assert_eq!(child_runs.load(Ordering::SeqCst), 2);
    Ok(())
}

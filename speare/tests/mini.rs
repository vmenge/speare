use flume::Receiver;
use speare::{
    mini::{root, OnErr, SpeareErr},
    Backoff, Limit,
};
use std::{
    fmt::Debug,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::time::{sleep, timeout, Duration};

const RECV_TIMEOUT: Duration = Duration::from_millis(500);
const QUIET_TIMEOUT: Duration = Duration::from_millis(150);

async fn recv_within<T>(rx: &Receiver<T>) -> T
where
    T: Send + 'static,
{
    timeout(RECV_TIMEOUT, rx.recv_async())
        .await
        .expect("timed out waiting for message")
        .expect("channel closed before delivering message")
}

async fn assert_no_message<T>(rx: &Receiver<T>)
where
    T: Debug + Send + 'static,
{
    match timeout(QUIET_TIMEOUT, rx.recv_async()).await {
        Err(_) => {}
        Ok(Err(_)) => {}
        Ok(Ok(msg)) => panic!("unexpected message: {msg:?}"),
    }
}

#[tokio::test]
async fn publish_delivers_to_subscriber() {
    // Arrange
    let root = root();
    let rx = root.subscribe::<u32>("numbers").unwrap();

    // Act
    root.publish("numbers", 42_u32).unwrap();

    // Assert
    assert_eq!(recv_within(&rx).await, 42);
}

#[tokio::test]
async fn publish_delivers_to_multiple_subscribers() {
    // Arrange
    let root = root();
    let rx_a = root.subscribe::<u32>("numbers").unwrap();
    let rx_b = root.subscribe::<u32>("numbers").unwrap();

    // Act
    root.publish("numbers", 42_u32).unwrap();

    // Assert
    assert_eq!(recv_within(&rx_a).await, 42);
    assert_eq!(recv_within(&rx_b).await, 42);
}

#[tokio::test]
async fn publish_to_empty_topic_is_ok() {
    // Arrange
    let root = root();

    // Act
    let result = root.publish("nobody-listens", 42_u32);

    // Assert
    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn subscribe_type_mismatch_returns_error() {
    // Arrange
    let root = root();
    let _rx = root.subscribe::<u32>("numbers").unwrap();

    // Act
    let err = root.subscribe::<String>("numbers").unwrap_err();

    // Assert
    assert_eq!(
        err,
        SpeareErr::TypeMismatch {
            topic: "numbers".to_string()
        }
    );
}

#[tokio::test]
async fn publish_type_mismatch_returns_error() {
    // Arrange
    let root = root();
    let _rx = root.subscribe::<u32>("numbers").unwrap();

    // Act
    let err = root
        .publish("numbers", "wrong type".to_string())
        .unwrap_err();

    // Assert
    assert_eq!(
        err,
        SpeareErr::TypeMismatch {
            topic: "numbers".to_string()
        }
    );
}

#[tokio::test]
async fn task_runs_once_with_async_closure() {
    // Arrange
    let root = root();
    let rx = root.subscribe::<u32>("done").unwrap();

    // Act
    root.task(async |ctx| {
        ctx.publish("done", 1_u32)?;
        Ok::<(), SpeareErr>(())
    })
    .unwrap();

    // Assert
    assert_eq!(recv_within(&rx).await, 1);
    assert_no_message(&rx).await;
}

#[tokio::test]
async fn task_with_passes_args_into_child_ctx() {
    // Arrange
    let root = root();
    let rx = root.subscribe::<u32>("args").unwrap();

    // Act
    root.task_with()
        .args(7_u32)
        .spawn(async |ctx| {
            ctx.publish("args", *ctx)?;
            Ok::<(), SpeareErr>(())
        })
        .unwrap();

    // Assert
    assert_eq!(recv_within(&rx).await, 7);
}

#[tokio::test]
async fn spawnwatch_reports_error_once_for_stop_policy() {
    // Arrange
    let root = root();
    let watch_rx = root
        .task_with()
        .on_err(OnErr::Stop)
        .spawnwatch(async |_ctx| Err::<(), &'static str>("boom"))
        .unwrap();

    // Act
    let (_task_id, err) = recv_within(&watch_rx).await;

    // Assert
    assert_eq!(err, "boom");
    assert_no_message(&watch_rx).await;
}

#[tokio::test]
async fn restart_retries_until_success() {
    // Arrange
    let root = root();
    let attempts = Arc::new(AtomicUsize::new(0));
    let (done_tx, done_rx) = flume::unbounded();

    let watch_rx = root
        .task_with()
        .args((attempts.clone(), done_tx.clone()))
        .on_err(OnErr::Restart {
            max: Limit::None,
            backoff: Backoff::None,
        })
        .spawnwatch(async |ctx| {
            let attempt = ctx.0.fetch_add(1, Ordering::SeqCst) + 1;

            if attempt < 3 {
                Err::<(), &'static str>("retry")
            } else {
                ctx.1.send(attempt).unwrap();
                Ok(())
            }
        })
        .unwrap();

    // Act
    let (task_id_a, err_a) = recv_within(&watch_rx).await;
    let (task_id_b, err_b) = recv_within(&watch_rx).await;

    // Assert
    assert_eq!(err_a, "retry");
    assert_eq!(err_b, "retry");
    assert_eq!(task_id_a, task_id_b);
    assert_eq!(recv_within(&done_rx).await, 3);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_no_message(&watch_rx).await;
}

#[tokio::test]
async fn restart_stops_after_max_restarts() {
    // Arrange
    let root = root();
    let attempts = Arc::new(AtomicUsize::new(0));

    let watch_rx = root
        .task_with()
        .args(attempts.clone())
        .on_err(OnErr::Restart {
            max: 2.into(),
            backoff: Backoff::None,
        })
        .spawnwatch(async |ctx| {
            ctx.fetch_add(1, Ordering::SeqCst);
            Err("retry")
        })
        .unwrap();

    // Act
    let (_, first) = recv_within(&watch_rx).await;
    let (_, second) = recv_within(&watch_rx).await;
    let (_, third) = recv_within(&watch_rx).await;

    // Assert
    assert_eq!(first, "retry");
    assert_eq!(second, "retry");
    assert_eq!(third, "retry");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_no_message(&watch_rx).await;
}

#[tokio::test]
async fn abort_children_stops_all_running_tasks() {
    // Arrange
    let root = root();
    let (done_tx, done_rx) = flume::unbounded::<u8>();

    for id in [1_u8, 2] {
        root.task_with()
            .args((done_tx.clone(), id))
            .spawn(async |ctx| {
                sleep(Duration::from_millis(80)).await;
                ctx.0.send(ctx.1).unwrap();
                Ok::<(), &'static str>(())
            })
            .unwrap();
    }

    // Act
    root.abort_children().unwrap();

    // Assert
    assert_no_message(&done_rx).await;
}

#[tokio::test]
async fn dropping_root_aborts_running_children() {
    // Arrange
    let (done_tx, done_rx) = flume::unbounded::<u8>();

    // Act
    {
        let root = root();

        root.task_with()
            .args(done_tx.clone())
            .spawn(async |ctx| {
                sleep(Duration::from_millis(80)).await;
                ctx.send(1).unwrap();
                Ok::<(), &'static str>(())
            })
            .unwrap();
    }

    // Assert
    assert_no_message(&done_rx).await;
}

use speare::{Actor, Backoff, Ctx, Limit, Node, Supervision};
use std::time::Duration;
use tokio::task;
use tokio::time::Instant;

mod sync_vec;
use sync_vec::SyncVec;

struct Failer;

impl Actor for Failer {
    type Props = SyncVec<Instant>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().push(Instant::now()).await;
        Err(())
    }
}

#[tokio::test(start_paused = true)]
async fn static_backoff_delays_each_restart_by_fixed_duration() {
    // Arrange
    let mut node = Node::default();
    let timestamps: SyncVec<Instant> = Default::default();
    let backoff_dur = Duration::from_secs(1);

    // Act
    node.actor::<Failer>(timestamps.clone())
        .supervision(Supervision::Restart {
            max: Limit::Amount(4),
            backoff: Backoff::Static(backoff_dur),
        })
        .spawn();

    tokio::time::sleep(Duration::from_secs(5)).await;
    task::yield_now().await;

    // Assert
    let ts = timestamps.clone_vec().await;
    assert_eq!(ts.len(), 4); // initial + 3 restarts

    for i in 1..ts.len() {
        let gap = ts[i] - ts[i - 1];
        assert_eq!(gap, backoff_dur);
    }
}

#[tokio::test(start_paused = true)]
async fn incremental_backoff_increases_delay_linearly() {
    // Arrange
    let mut node = Node::default();
    let timestamps: SyncVec<Instant> = Default::default();

    let step = Duration::from_millis(100);
    let min = Duration::from_millis(50);
    let max = Duration::from_secs(1);

    // Act
    node.actor::<Failer>(timestamps.clone())
        .supervision(Supervision::Restart {
            max: Limit::Amount(5),
            backoff: Backoff::Incremental { min, max, step },
        })
        .spawn();

    tokio::time::sleep(Duration::from_secs(5)).await;
    task::yield_now().await;

    // Assert
    let ts = timestamps.clone_vec().await;
    assert_eq!(ts.len(), 5); // initial + 4 restarts

    // Backoff formula: wait = max(min, min(max, step * (current_restarts + 1)))
    // restart 0 (0 prior restarts): step * 1 = 100ms, clamped to [50ms, 1s] = 100ms
    // restart 1 (1 prior restart):  step * 2 = 200ms, clamped to [50ms, 1s] = 200ms
    // restart 2 (2 prior restarts): step * 3 = 300ms, clamped to [50ms, 1s] = 300ms
    // restart 3 (3 prior restarts): step * 4 = 400ms, clamped to [50ms, 1s] = 400ms
    let expected_gaps = [
        Duration::from_millis(100),
        Duration::from_millis(200),
        Duration::from_millis(300),
        Duration::from_millis(400),
    ];

    for (i, expected) in expected_gaps.iter().enumerate() {
        let gap = ts[i + 1] - ts[i];
        assert_eq!(gap, *expected, "gap {i} mismatch");
    }
}

#[tokio::test(start_paused = true)]
async fn incremental_backoff_clamps_to_max() {
    // Arrange
    let mut node = Node::default();
    let timestamps: SyncVec<Instant> = Default::default();

    let step = Duration::from_millis(500);
    let min = Duration::from_millis(100);
    let max = Duration::from_millis(800);

    // Act
    node.actor::<Failer>(timestamps.clone())
        .supervision(Supervision::Restart {
            max: Limit::Amount(5),
            backoff: Backoff::Incremental { min, max, step },
        })
        .spawn();

    tokio::time::sleep(Duration::from_secs(10)).await;
    task::yield_now().await;

    // Assert
    let ts = timestamps.clone_vec().await;
    assert_eq!(ts.len(), 5);

    // restart 0: step * 1 = 500ms, clamped to [100ms, 800ms] = 500ms
    // restart 1: step * 2 = 1000ms, clamped to [100ms, 800ms] = 800ms
    // restart 2: step * 3 = 1500ms, clamped to [100ms, 800ms] = 800ms
    // restart 3: step * 4 = 2000ms, clamped to [100ms, 800ms] = 800ms
    let expected_gaps = [
        Duration::from_millis(500),
        Duration::from_millis(800),
        Duration::from_millis(800),
        Duration::from_millis(800),
    ];

    for (i, expected) in expected_gaps.iter().enumerate() {
        let gap = ts[i + 1] - ts[i];
        assert_eq!(gap, *expected, "gap {i} mismatch");
    }
}

#[tokio::test(start_paused = true)]
async fn no_backoff_restarts_immediately() {
    // Arrange
    let mut node = Node::default();
    let timestamps: SyncVec<Instant> = Default::default();

    // Act
    node.actor::<Failer>(timestamps.clone())
        .supervision(Supervision::Restart {
            max: Limit::Amount(3),
            backoff: Backoff::None,
        })
        .spawn();

    task::yield_now().await;
    task::yield_now().await;
    task::yield_now().await;

    // Assert
    let ts = timestamps.clone_vec().await;
    assert_eq!(ts.len(), 3); // initial + 2 restarts

    for i in 1..ts.len() {
        let gap = ts[i] - ts[i - 1];
        assert_eq!(gap, Duration::ZERO);
    }
}

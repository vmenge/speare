mod sync_vec;

use speare::{Actor, Ctx, Node};
use std::time::Duration;
use sync_vec::SyncVec;
use tokio::time;

#[derive(Debug, PartialEq, Clone)]
enum Msg {
    Tick,
    Manual(u32),
}

struct Ticker;

impl Actor for Ticker {
    type Props = SyncVec<Msg>;
    type Msg = Msg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Ticker)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().push(msg).await;
        Ok(())
    }
}

#[tokio::test]
async fn interval_sends_ticks_to_actor() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let handle = node
        .actor::<Ticker>(recvd.clone())
        .interval(time::interval(Duration::from_millis(10)), || Msg::Tick)
        .spawn();

    // Act
    time::sleep(Duration::from_millis(35)).await;
    handle.stop();
    time::sleep(Duration::from_millis(1)).await;

    // Assert
    let msgs = recvd.clone_vec().await;
    assert!(msgs.len() >= 3, "expected at least 3 ticks, got {}", msgs.len());
    assert!(msgs.iter().all(|m| *m == Msg::Tick));
}

#[tokio::test]
async fn interval_and_handle_messages_both_received() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let handle = node
        .actor::<Ticker>(recvd.clone())
        .interval(time::interval(Duration::from_millis(10)), || Msg::Tick)
        .spawn();

    // Act
    time::sleep(Duration::from_millis(25)).await;
    handle.send(Msg::Manual(1));
    time::sleep(Duration::from_millis(1)).await;
    handle.stop();
    time::sleep(Duration::from_millis(1)).await;

    // Assert
    let msgs = recvd.clone_vec().await;
    assert!(msgs.contains(&Msg::Manual(1)));
    assert!(msgs.contains(&Msg::Tick));
}

#[tokio::test]
async fn multiple_intervals() {
    // Arrange
    #[derive(Debug, PartialEq, Clone)]
    enum MultiMsg {
        Fast,
        Slow,
    }

    struct Multi;

    impl Actor for Multi {
        type Props = SyncVec<MultiMsg>;
        type Msg = MultiMsg;
        type Err = ();

        async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(Multi)
        }

        async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
            ctx.props().push(msg).await;
            Ok(())
        }
    }

    let mut node = Node::default();
    let recvd: SyncVec<MultiMsg> = Default::default();
    let handle = node
        .actor::<Multi>(recvd.clone())
        .interval(time::interval(Duration::from_millis(10)), || MultiMsg::Fast)
        .interval(time::interval(Duration::from_millis(30)), || MultiMsg::Slow)
        .spawn();

    // Act
    time::sleep(Duration::from_millis(65)).await;
    handle.stop();
    time::sleep(Duration::from_millis(1)).await;

    // Assert
    let msgs = recvd.clone_vec().await;
    let fast_count = msgs.iter().filter(|m| **m == MultiMsg::Fast).count();
    let slow_count = msgs.iter().filter(|m| **m == MultiMsg::Slow).count();
    assert!(fast_count >= 5, "expected at least 5 fast ticks, got {}", fast_count);
    assert!(slow_count >= 2, "expected at least 2 slow ticks, got {}", slow_count);
}

#[tokio::test]
async fn interval_stops_when_actor_stops() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let handle = node
        .actor::<Ticker>(recvd.clone())
        .interval(time::interval(Duration::from_millis(10)), || Msg::Tick)
        .spawn();

    // Act
    time::sleep(Duration::from_millis(25)).await;
    handle.stop();
    time::sleep(Duration::from_millis(1)).await;
    let count_at_stop = recvd.clone_vec().await.len();

    time::sleep(Duration::from_millis(50)).await;
    let count_after_wait = recvd.clone_vec().await.len();

    // Assert
    assert!(!handle.is_alive());
    assert_eq!(count_at_stop, count_after_wait);
}

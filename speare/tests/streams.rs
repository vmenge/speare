mod sync_vec;

use futures_core::Stream;
use speare::{Actor, Ctx, Node};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use sync_vec::SyncVec;
use tokio::{task, time};

struct ChannelStream<T> {
    rx: flume::Receiver<T>,
}

impl<T> Stream for ChannelStream<T> {
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        match self.rx.try_recv() {
            Ok(item) => Poll::Ready(Some(item)),
            Err(flume::TryRecvError::Empty) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(flume::TryRecvError::Disconnected) => Poll::Ready(None),
        }
    }
}

fn channel_stream<T>() -> (flume::Sender<T>, ChannelStream<T>) {
    let (tx, rx) = flume::unbounded();
    (tx, ChannelStream { rx })
}

#[derive(Debug, PartialEq, Clone)]
enum Msg {
    A(u32),
    B(u32),
}

struct Collector;

impl Actor for Collector {
    type Props = SyncVec<Msg>;
    type Msg = Msg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Collector)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().push(msg).await;
        Ok(())
    }
}

#[tokio::test]
async fn actor_receives_messages_from_stream() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let (tx, stream) = channel_stream();
    let _handle = node.actor::<Collector>(recvd.clone()).stream(stream).spawn();

    // Act
    tx.send(Msg::A(1)).unwrap();
    tx.send(Msg::A(2)).unwrap();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![Msg::A(1), Msg::A(2)]);
}

#[tokio::test]
async fn actor_receives_from_both_handle_and_stream() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let (tx, stream) = channel_stream();
    let handle = node.actor::<Collector>(recvd.clone()).stream(stream).spawn();

    // Act
    handle.send(Msg::A(1));
    tx.send(Msg::B(2)).unwrap();
    task::yield_now().await;

    // Assert
    let msgs = recvd.clone_vec().await;
    assert_eq!(msgs.len(), 2);
    assert!(msgs.contains(&Msg::A(1)));
    assert!(msgs.contains(&Msg::B(2)));
}

#[tokio::test]
async fn actor_receives_from_multiple_streams() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let (tx1, stream1) = channel_stream();
    let (tx2, stream2) = channel_stream();
    let _handle = node
        .actor::<Collector>(recvd.clone())
        .stream(stream1)
        .stream(stream2)
        .spawn();

    // Act
    tx1.send(Msg::A(1)).unwrap();
    tx2.send(Msg::B(2)).unwrap();
    task::yield_now().await;

    // Assert
    let msgs = recvd.clone_vec().await;
    assert_eq!(msgs.len(), 2);
    assert!(msgs.contains(&Msg::A(1)));
    assert!(msgs.contains(&Msg::B(2)));
}

#[tokio::test]
async fn actor_stops_cleanly_with_active_stream() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let (tx, stream) = channel_stream();
    let handle = node.actor::<Collector>(recvd.clone()).stream(stream).spawn();

    tx.send(Msg::A(1)).unwrap();
    task::yield_now().await;

    // Act
    handle.stop();
    time::sleep(std::time::Duration::from_millis(1)).await;

    // Assert
    assert!(!handle.is_alive());
    let _ = tx.send(Msg::A(99));
}

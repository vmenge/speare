mod sync_vec;

use futures_core::Stream;
use speare::{Actor, Ctx, Node, SourceSet, Sources};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use sync_vec::SyncVec;
use tokio::{task, time};

struct ChannelStream<T> {
    rx: flume::Receiver<T>,
}

impl<T> From<flume::Receiver<T>> for ChannelStream<T> {
    fn from(rx: flume::Receiver<T>) -> Self {
        ChannelStream { rx }
    }
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

#[derive(Debug, PartialEq, Clone)]
enum Msg {
    A(u32),
    B(u32),
}

struct CollectorProps {
    recvd: SyncVec<Msg>,
    stream_rx: flume::Receiver<Msg>,
}

struct Collector;

impl Actor for Collector {
    type Props = CollectorProps;
    type Msg = Msg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Collector)
    }

    async fn sources(&self, ctx: &Ctx<Self>) -> Result<impl Sources<Self>, Self::Err> {
        let ch_stream = ChannelStream::from(ctx.props().stream_rx.clone());
        let sources = SourceSet::new().stream(ch_stream);

        Ok(sources)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().recvd.push(msg).await;
        Ok(())
    }
}

struct MultiCollectorProps {
    recvd: SyncVec<Msg>,
    stream_rx1: flume::Receiver<Msg>,
    stream_rx2: flume::Receiver<Msg>,
}

struct MultiCollector;

impl Actor for MultiCollector {
    type Props = MultiCollectorProps;
    type Msg = Msg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(MultiCollector)
    }

    async fn sources(&self, ctx: &Ctx<Self>) -> Result<impl Sources<Self>, Self::Err> {
        Ok(SourceSet::new()
            .stream(ChannelStream::from(ctx.props().stream_rx1.clone()))
            .stream(ChannelStream::from(ctx.props().stream_rx2.clone())))
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().recvd.push(msg).await;
        Ok(())
    }
}

#[tokio::test]
async fn actor_receives_messages_from_stream() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let (tx, rx) = flume::unbounded();
    let _handle = node
        .actor::<Collector>(CollectorProps {
            recvd: recvd.clone(),
            stream_rx: rx,
        })
        .spawn();

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
    let (tx, rx) = flume::unbounded();
    let handle = node
        .actor::<Collector>(CollectorProps {
            recvd: recvd.clone(),
            stream_rx: rx,
        })
        .spawn();

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
    let (tx1, rx1) = flume::unbounded();
    let (tx2, rx2) = flume::unbounded();
    let _handle = node
        .actor::<MultiCollector>(MultiCollectorProps {
            recvd: recvd.clone(),
            stream_rx1: rx1,
            stream_rx2: rx2,
        })
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
    let (tx, rx) = flume::unbounded();
    let handle = node
        .actor::<Collector>(CollectorProps {
            recvd: recvd.clone(),
            stream_rx: rx,
        })
        .spawn();

    tx.send(Msg::A(1)).unwrap();
    task::yield_now().await;

    // Act
    handle.stop();
    time::sleep(std::time::Duration::from_millis(1)).await;

    // Assert
    assert!(!handle.is_alive());
    let _ = tx.send(Msg::A(99));
}

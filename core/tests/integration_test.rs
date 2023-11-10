use std::time::Duration;

use flume::Sender;
use speare::*;
use tokio::time;

#[derive(Clone, Debug)]
struct DoubleCount;

#[derive(Clone, Debug)]
struct IncCount;

#[derive(Default, Debug)]
struct Counter {
    count: u64,
}

#[async_trait]
impl Process for Counter {
    type Error = ();

    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<DoubleCount>().await;
        evt.subscribe::<IncCount>().await;
    }

    async fn on_init(&mut self, _ctx: &Ctx<Self>) {
        if self.count == 10 {
            self.count = 0;
        }
    }
}

#[async_trait]
impl Handler<IncCount> for Counter {
    type Ok = u64;
    type Err = ();

    async fn handle(&mut self, _msg: IncCount, _ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.count += 1;
        reply(self.count)
    }
}

#[async_trait]
impl Handler<DoubleCount> for Counter {
    type Ok = u64;
    type Err = ();

    async fn handle(&mut self, _msg: DoubleCount, _ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.count *= 2;
        reply(self.count)
    }
}

#[tokio::test]
async fn init_is_called() {
    // Arrange
    let node = Node::default();

    // Act
    let counter_pid = node.spawn(Counter { count: 10 }).await;
    let result = node.ask(&counter_pid, IncCount).await.unwrap();

    // Assert
    assert_eq!(result, 1);
}

#[tokio::test]
async fn on_exit_is_called() {
    struct MyProc {
        tx: Sender<u8>,
    }

    #[async_trait]
    impl Process for MyProc {
        type Error = ();

        async fn on_exit(&mut self, _ctx: &Ctx<Self>) {
            self.tx.send(1).unwrap();
        }
    }

    // Arrange
    let node = Node::default();
    let (tx, rx) = flume::unbounded();
    let pid = node.spawn(MyProc { tx }).await;

    // Act
    node.exit(&pid, ExitReason::Shutdown).await;
    time::sleep(Duration::ZERO).await;

    // Assert
    let recvd = rx.try_recv().unwrap();
    assert_eq!(1, recvd);
}

#[tokio::test]
async fn tells_and_asks_a_process_synchronously() {
    // Arrange
    let node = Node::default();
    let counter_pid = node.spawn(Counter::default()).await;

    // Act
    for _ in 0..10 {
        node.tell(&counter_pid, IncCount).await;
    }

    let result = node.ask(&counter_pid, IncCount).await.unwrap();

    // Assert
    assert_eq!(result, 11);
}

#[tokio::test]
async fn pub_sub_works() {
    // Arrange
    let node = Node::default();
    let counter_pid = node.spawn(Counter::default()).await;

    // Act
    node.publish(IncCount).await;
    let result = node.ask(&counter_pid, IncCount).await.unwrap();

    // Assert
    assert_eq!(result, 2);
}

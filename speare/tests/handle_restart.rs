use derive_more::From;
use speare::{Actor, Ctx, ExitReason, Node, Request};
use tokio::task;

mod sync_vec;
use sync_vec::SyncVec;

struct Counter {
    count: u32,
}

#[derive(From)]
enum CounterMsg {
    Inc,
    GetCount(Request<(), u32>),
}

impl Actor for Counter {
    type Props = SyncVec<String>;
    type Msg = CounterMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().push("init".into()).await;
        Ok(Self { count: 0 })
    }

    async fn exit(_: Option<Self>, reason: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        let reason_str = match reason {
            ExitReason::Handle => "handle",
            ExitReason::Parent => "parent",
            ExitReason::Err(_) => "err",
        };

        ctx.props().push(format!("exit:{reason_str}")).await;
    }

    async fn handle(&mut self, msg: CounterMsg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            CounterMsg::Inc => self.count += 1,
            CounterMsg::GetCount(req) => req.reply(self.count),
        }

        Ok(())
    }
}

#[tokio::test]
async fn restart_resets_actor_state() {
    // Arrange
    let mut node = Node::default();
    let log: SyncVec<String> = Default::default();
    let handle = node.actor::<Counter>(log.clone()).spawn();
    task::yield_now().await;

    handle.send(CounterMsg::Inc);
    handle.send(CounterMsg::Inc);
    let count: u32 = handle.req(()).await.unwrap();
    assert_eq!(count, 2);

    // Act
    handle.restart();
    task::yield_now().await;

    // Assert
    let count: u32 = handle.req(()).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn restart_keeps_actor_alive() {
    // Arrange
    let mut node = Node::default();
    let log: SyncVec<String> = Default::default();
    let handle = node.actor::<Counter>(log.clone()).spawn();
    task::yield_now().await;
    assert!(handle.is_alive());

    // Act
    handle.restart();
    task::yield_now().await;

    // Assert
    assert!(handle.is_alive());
}

#[tokio::test]
async fn restart_calls_exit_then_init() {
    // Arrange
    let mut node = Node::default();
    let log: SyncVec<String> = Default::default();
    let handle = node.actor::<Counter>(log.clone()).spawn();
    task::yield_now().await;

    // Act
    handle.restart();
    task::yield_now().await;

    // Assert
    assert_eq!(log.clone_vec().await, vec!["init", "exit:handle", "init"]);
}

#[tokio::test]
async fn restart_allows_continued_messaging() {
    // Arrange
    let mut node = Node::default();
    let log: SyncVec<String> = Default::default();
    let handle = node.actor::<Counter>(log.clone()).spawn();
    task::yield_now().await;

    // Act
    handle.restart();
    task::yield_now().await;

    handle.send(CounterMsg::Inc);
    handle.send(CounterMsg::Inc);
    handle.send(CounterMsg::Inc);

    // Assert
    let count: u32 = handle.req(()).await.unwrap();
    assert_eq!(count, 3);
}

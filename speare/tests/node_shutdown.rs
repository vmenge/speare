use speare::{Actor, Ctx, ExitReason, Node};
use tokio::task;

mod sync_vec;
use sync_vec::SyncVec;

struct Worker;

impl Actor for Worker {
    type Props = SyncVec<String>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().push("init".into()).await;
        Ok(Worker)
    }

    async fn exit(_: Option<Self>, _: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        ctx.props().push("exit".into()).await;
    }
}

#[tokio::test]
async fn shutdown_stops_all_children() {
    // Arrange
    let mut node = Node::default();
    let h1 = node.actor::<Worker>(Default::default()).spawn();
    let h2 = node.actor::<Worker>(Default::default()).spawn();
    task::yield_now().await;
    assert!(h1.is_alive());
    assert!(h2.is_alive());

    // Act
    node.shutdown().await;

    // Assert
    assert!(!h1.is_alive());
    assert!(!h2.is_alive());
}

#[tokio::test]
async fn shutdown_calls_exit_on_all_children() {
    // Arrange
    let mut node = Node::default();
    let log: SyncVec<String> = Default::default();
    node.actor::<Worker>(log.clone()).spawn();
    node.actor::<Worker>(log.clone()).spawn();
    task::yield_now().await;

    // Act
    node.shutdown().await;

    // Assert
    let events = log.clone_vec().await;
    assert_eq!(events.iter().filter(|e| *e == "init").count(), 2);
    assert_eq!(events.iter().filter(|e| *e == "exit").count(), 2);
}

struct Parent;

impl Actor for Parent {
    type Props = SyncVec<String>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.actor::<Worker>(ctx.props().clone()).spawn();
        ctx.actor::<Worker>(ctx.props().clone()).spawn();
        Ok(Parent)
    }
}

#[tokio::test]
async fn shutdown_stops_nested_children() {
    // Arrange
    let mut node = Node::default();
    let log: SyncVec<String> = Default::default();
    node.actor::<Parent>(log.clone()).spawn();
    task::yield_now().await;

    // Act
    node.shutdown().await;

    // Assert - both grandchildren should have their exit called
    let events = log.clone_vec().await;
    assert_eq!(events.iter().filter(|e| *e == "init").count(), 2);
    assert_eq!(events.iter().filter(|e| *e == "exit").count(), 2);
}

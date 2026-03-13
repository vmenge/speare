mod sync_vec;

use speare::{Actor, Backoff, Ctx, Handle, Limit, Node, Supervision};
use sync_vec::SyncVec;
use tokio::task;

struct Child;

enum ChildMsg {
    Fail,
}

impl Actor for Child {
    type Props = ();
    type Msg = ChildMsg;
    type Err = String;

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Child)
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ChildMsg::Fail => Err("child failed".to_string()),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
struct ChildFailed(String);

enum ParentMsg {
    ChildFailed(ChildFailed),
}

struct ParentProps {
    supervision: Supervision,
    recvd: SyncVec<ChildFailed>,
}

struct Parent;

impl Actor for Parent {
    type Props = ParentProps;
    type Msg = ParentMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let supervision = ctx.props().supervision;

        ctx.actor::<Child>(())
            .supervision(supervision)
            .watch(|err| ParentMsg::ChildFailed(ChildFailed(err.clone())))
            .spawn_named("child")
            .unwrap();

        Ok(Parent)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ParentMsg::ChildFailed(f) => ctx.props().recvd.push(f).await,
        }

        Ok(())
    }
}

#[tokio::test]
async fn fires_on_supervision_stop() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<ChildFailed>::default();
    node.actor::<Parent>(ParentProps {
        supervision: Supervision::Stop,
        recvd: recvd.clone(),
    })
    .spawn();

    task::yield_now().await;
    let child: Handle<ChildMsg> = node.get_handle("child").unwrap();

    // Act
    child.send(ChildMsg::Fail);
    task::yield_now().await;

    // Assert
    assert_eq!(
        recvd.clone_vec().await,
        vec![ChildFailed("child failed".to_string())]
    );
}

#[tokio::test]
async fn fires_when_max_restarts_reached() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<ChildFailed>::default();
    node.actor::<Parent>(ParentProps {
        supervision: Supervision::Restart {
            max: 2.into(),
            backoff: Backoff::None,
        },
        recvd: recvd.clone(),
    })
    .spawn();

    task::yield_now().await;
    let child: Handle<ChildMsg> = node.get_handle("child").unwrap();

    // Act & Assert

    // 1st fail -> restart (watch should NOT fire)
    child.send(ChildMsg::Fail);
    task::yield_now().await;
    assert!(recvd.clone_vec().await.is_empty());

    // 2nd fail -> max reached, terminates (watch SHOULD fire)
    child.send(ChildMsg::Fail);
    task::yield_now().await;
    assert_eq!(
        recvd.clone_vec().await,
        vec![ChildFailed("child failed".to_string())]
    );
}

#[tokio::test]
async fn does_not_fire_on_restart() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<ChildFailed>::default();
    node.actor::<Parent>(ParentProps {
        supervision: Supervision::Restart {
            max: Limit::None,
            backoff: Backoff::None,
        },
        recvd: recvd.clone(),
    })
    .spawn();

    task::yield_now().await;
    let child: Handle<ChildMsg> = node.get_handle("child").unwrap();

    // Act
    child.send(ChildMsg::Fail);
    task::yield_now().await;

    // Assert
    assert!(child.is_alive());
    assert!(recvd.clone_vec().await.is_empty());
}

#[tokio::test]
async fn does_not_fire_on_handle_stop() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<ChildFailed>::default();
    node.actor::<Parent>(ParentProps {
        supervision: Supervision::Stop,
        recvd: recvd.clone(),
    })
    .spawn();

    task::yield_now().await;
    let child: Handle<ChildMsg> = node.get_handle("child").unwrap();

    // Act
    child.stop();
    task::yield_now().await;

    // Assert
    assert!(!child.is_alive());
    assert!(recvd.clone_vec().await.is_empty());
}

#[tokio::test]
async fn does_not_fire_on_resume() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<ChildFailed>::default();
    node.actor::<Parent>(ParentProps {
        supervision: Supervision::Resume,
        recvd: recvd.clone(),
    })
    .spawn();

    task::yield_now().await;
    let child: Handle<ChildMsg> = node.get_handle("child").unwrap();

    // Act
    child.send(ChildMsg::Fail);
    task::yield_now().await;

    // Assert
    assert!(child.is_alive());
    assert!(recvd.clone_vec().await.is_empty());
}

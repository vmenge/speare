mod sync_vec;

use speare::{Actor, Backoff, Ctx, Handle, Limit, Node, Request, Supervision};
use std::time::Duration;
use sync_vec::SyncVec;
use tokio::time;

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
    GetChild(Request<(), Handle<ChildMsg>>),
    ChildFailed(ChildFailed),
}

struct ParentProps {
    supervision: Supervision,
    recvd: SyncVec<ChildFailed>,
}

struct Parent {
    child: Handle<ChildMsg>,
}

impl Actor for Parent {
    type Props = ParentProps;
    type Msg = ParentMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let supervision = ctx.props().supervision;

        let child = ctx
            .actor::<Child>(())
            .supervision(supervision)
            .watch(|err| ParentMsg::ChildFailed(ChildFailed(err.clone())))
            .spawn();

        Ok(Parent { child })
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ParentMsg::GetChild(req) => req.reply(self.child.clone()),
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
    let parent = node
        .actor::<Parent>(ParentProps {
            supervision: Supervision::Stop,
            recvd: recvd.clone(),
        })
        .spawn();

    let child: Handle<ChildMsg> = parent.reqw(ParentMsg::GetChild, ()).await.unwrap();

    // Act
    child.send(ChildMsg::Fail);
    time::sleep(Duration::from_millis(10)).await;

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
    let parent = node
        .actor::<Parent>(ParentProps {
            supervision: Supervision::Restart {
                max: 2.into(),
                backoff: Backoff::None,
            },
            recvd: recvd.clone(),
        })
        .spawn();

    let child: Handle<ChildMsg> = parent.reqw(ParentMsg::GetChild, ()).await.unwrap();

    // Act & Assert

    // 1st fail -> restart (watch should NOT fire)
    child.send(ChildMsg::Fail);
    time::sleep(Duration::from_millis(10)).await;
    assert!(recvd.clone_vec().await.is_empty());

    // 2nd fail -> max reached, terminates (watch SHOULD fire)
    child.send(ChildMsg::Fail);
    time::sleep(Duration::from_millis(10)).await;
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
    let parent = node
        .actor::<Parent>(ParentProps {
            supervision: Supervision::Restart {
                max: Limit::None,
                backoff: Backoff::None,
            },
            recvd: recvd.clone(),
        })
        .spawn();

    let child: Handle<ChildMsg> = parent.reqw(ParentMsg::GetChild, ()).await.unwrap();

    // Act
    child.send(ChildMsg::Fail);
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert!(child.is_alive());
    assert!(recvd.clone_vec().await.is_empty());
}

#[tokio::test]
async fn does_not_fire_on_handle_stop() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<ChildFailed>::default();
    let parent = node
        .actor::<Parent>(ParentProps {
            supervision: Supervision::Stop,
            recvd: recvd.clone(),
        })
        .spawn();

    let child: Handle<ChildMsg> = parent.reqw(ParentMsg::GetChild, ()).await.unwrap();

    // Act
    child.stop();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert!(!child.is_alive());
    assert!(recvd.clone_vec().await.is_empty());
}

#[tokio::test]
async fn does_not_fire_on_resume() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<ChildFailed>::default();
    let parent = node
        .actor::<Parent>(ParentProps {
            supervision: Supervision::Resume,
            recvd: recvd.clone(),
        })
        .spawn();

    let child: Handle<ChildMsg> = parent.reqw(ParentMsg::GetChild, ()).await.unwrap();

    // Act
    child.send(ChildMsg::Fail);
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert!(child.is_alive());
    assert!(recvd.clone_vec().await.is_empty());
}

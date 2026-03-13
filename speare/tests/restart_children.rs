use derive_more::From;
use speare::{Actor, Backoff, Ctx, Handle, Limit, Node, Request, Supervision};
use tokio::task;

struct Child {
    count: u32,
}

#[derive(From)]
enum ChildMsg {
    Inc,
    GetCount(Request<(), u32>),
}

impl Actor for Child {
    type Props = ();
    type Msg = ChildMsg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Self { count: 0 })
    }

    async fn handle(&mut self, msg: ChildMsg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ChildMsg::Inc => self.count += 1,
            ChildMsg::GetCount(req) => req.reply(self.count),
        }

        Ok(())
    }
}

struct Parent {
    child_a: Handle<ChildMsg>,
    child_b: Handle<ChildMsg>,
}

#[derive(From)]
enum ParentMsg {
    RestartChildren,
    GetChildren(Request<(), (Handle<ChildMsg>, Handle<ChildMsg>)>),
}

impl Actor for Parent {
    type Props = ();
    type Msg = ParentMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Parent {
            child_a: ctx
                .actor::<Child>(())
                .supervision(Supervision::Restart {
                    max: Limit::None,
                    backoff: Backoff::None,
                })
                .spawn(),
            child_b: ctx
                .actor::<Child>(())
                .supervision(Supervision::Restart {
                    max: Limit::None,
                    backoff: Backoff::None,
                })
                .spawn(),
        })
    }

    async fn handle(&mut self, msg: ParentMsg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ParentMsg::RestartChildren => ctx.restart_children(),
            ParentMsg::GetChildren(req) => {
                req.reply((self.child_a.clone(), self.child_b.clone()));
            }
        }

        Ok(())
    }
}

#[tokio::test]
async fn restart_children_resets_all_children_state() {
    // Arrange
    let mut node = Node::default();
    let parent = node.actor::<Parent>(()).spawn();

    let (child_a, child_b) = parent.req(()).await.unwrap();

    child_a.send(ChildMsg::Inc);
    child_a.send(ChildMsg::Inc);
    child_b.send(ChildMsg::Inc);
    task::yield_now().await;

    let count_a: u32 = child_a.req(()).await.unwrap();
    let count_b: u32 = child_b.req(()).await.unwrap();
    assert_eq!(count_a, 2);
    assert_eq!(count_b, 1);

    // Act
    parent.send(ParentMsg::RestartChildren);
    task::yield_now().await;

    // Assert - counts should be reset to 0 after restart
    let count_a: u32 = child_a.req(()).await.unwrap();
    let count_b: u32 = child_b.req(()).await.unwrap();
    assert_eq!(count_a, 0);
    assert_eq!(count_b, 0);
}

#[tokio::test]
async fn restart_children_keeps_children_alive() {
    // Arrange
    let mut node = Node::default();
    let parent = node.actor::<Parent>(()).spawn();

    let (child_a, child_b) = parent.req(()).await.unwrap();
    assert!(child_a.is_alive());
    assert!(child_b.is_alive());

    // Act
    parent.send(ParentMsg::RestartChildren);
    task::yield_now().await;

    // Assert
    assert!(child_a.is_alive());
    assert!(child_b.is_alive());
}

#[tokio::test]
async fn restart_children_allows_continued_messaging() {
    // Arrange
    let mut node = Node::default();
    let parent = node.actor::<Parent>(()).spawn();

    let (child_a, child_b) = parent.req(()).await.unwrap();

    // Act - restart then send new messages
    parent.send(ParentMsg::RestartChildren);
    task::yield_now().await;

    child_a.send(ChildMsg::Inc);
    child_b.send(ChildMsg::Inc);
    child_b.send(ChildMsg::Inc);
    child_b.send(ChildMsg::Inc);

    // Assert - new messages processed after restart
    let count_a: u32 = child_a.req(()).await.unwrap();
    let count_b: u32 = child_b.req(()).await.unwrap();
    assert_eq!(count_a, 1);
    assert_eq!(count_b, 3);
}

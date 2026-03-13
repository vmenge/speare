use derive_more::From;
use speare::{Actor, Ctx, ExitReason, Handle, Node, Request, Supervision};
use tokio::task;
mod sync_vec;

struct Child {
    count: u32,
}

#[derive(From)]
enum ChildMsg {
    Fail,
    Count,
    GetCount(Request<(), u32>),
}

type Id = u32;

impl Actor for Child {
    type Props = Id;
    type Msg = ChildMsg;
    type Err = Id;

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Self { count: 0 })
    }

    async fn handle(&mut self, msg: ChildMsg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ChildMsg::Fail => return Err(*ctx.props()),
            ChildMsg::Count => self.count += 1,
            ChildMsg::GetCount(req) => req.reply(self.count),
        }

        Ok(())
    }

    async fn exit(_: Option<Self>, reason: ExitReason<Self>, _: &mut Ctx<Self>) {
        println!("Child exiting. {:?}", reason);
    }
}

mod one_for_one {
    use super::*;
    use speare::{Backoff, Limit};

    struct MaxResetAmount;

    impl Actor for MaxResetAmount {
        type Props = ();
        type Msg = ();
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.actor::<Child>(0)
                .supervision(Supervision::Restart {
                    max: 3.into(),
                    backoff: Backoff::None,
                })
                .spawn_named("child")
                .unwrap();

            Ok(MaxResetAmount)
        }

        async fn handle(&mut self, _: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn reaches_max_reset_limit_and_shuts_down_actor() {
        // Arrange
        let mut node = Node::default();
        node.actor::<MaxResetAmount>(()).spawn();
        task::yield_now().await;

        let child: Handle<ChildMsg> = node.get_handle("child").unwrap();
        let kill = || async {
            child.send(ChildMsg::Fail);
            task::yield_now().await; // wait for actor to be killed
        };

        // Act & Assert

        // No restarts, should be alive
        assert!(child.is_alive());

        kill().await;
        // 1 restart, should be alive
        assert!(child.is_alive());

        kill().await;
        // 2 restarts, should be alive
        assert!(child.is_alive());

        kill().await;
        // 3 restarts, should be dead
        assert!(!child.is_alive());
    }

    #[tokio::test]
    async fn using_node_as_parent_reaches_max_reset_limit_and_shuts_down_actor() {
        // Arrange
        let mut node = Node::default();
        let child = node
            .actor::<Child>(0)
            .supervision(Supervision::Restart {
                max: 3.into(),
                backoff: Backoff::None,
            })
            .spawn();

        let kill = || async {
            child.send(ChildMsg::Fail);
            task::yield_now().await; // wait for actor to be killed
        };

        // Act & Assert

        // No restarts, should be alive
        assert!(child.is_alive());

        kill().await;
        // 1 restart, should be alive
        assert!(child.is_alive());

        kill().await;
        // 2 restarts, should be alive
        assert!(child.is_alive());

        kill().await;
        // 3 restarts, should be dead
        assert!(!child.is_alive());
    }

    struct Parent;

    impl Actor for Parent {
        type Props = ();
        type Msg = ();
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.actor::<Child>(0)
                .supervision(Supervision::Resume)
                .spawn_named("child-0")
                .unwrap();

            ctx.actor::<Child>(1)
                .supervision(Supervision::Restart {
                    max: Limit::None,
                    backoff: Backoff::None,
                })
                .spawn_named("child-1")
                .unwrap();

            ctx.actor::<Child>(2)
                .supervision(Supervision::Stop)
                .spawn_named("child-2")
                .unwrap();

            Ok(Parent)
        }

        async fn handle(&mut self, _: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn all_different_strategies_work() {
        // Arrange
        let mut node = Node::default();
        node.actor::<Parent>(()).spawn();
        task::yield_now().await;

        let child0: Handle<ChildMsg> = node.get_handle("child-0").unwrap();
        let child1: Handle<ChildMsg> = node.get_handle("child-1").unwrap();
        let child2: Handle<ChildMsg> = node.get_handle("child-2").unwrap();

        child0.send(ChildMsg::Count);
        child1.send(ChildMsg::Count);
        child1.send(ChildMsg::Count);
        child2.send(ChildMsg::Count);
        child2.send(ChildMsg::Count);
        child2.send(ChildMsg::Count);

        let counts =
            || async { tokio::try_join!(child0.req(()), child1.req(()), child2.req(()),).unwrap() };

        assert_eq!(counts().await, (1, 2, 3));

        // Act & Assert

        // Fail child0, all counts should be unaffected
        child0.send(ChildMsg::Fail);
        task::yield_now().await;

        assert_eq!(counts().await, (1, 2, 3));

        // Fail child1, its count should go back to 0. Other counts should be unaffected.
        child1.send(ChildMsg::Fail);
        task::yield_now().await;

        assert_eq!(counts().await, (1, 0, 3));

        // Fail child2, Actor should stop. Other counts should remain unaffected.
        child2.send(ChildMsg::Fail);
        task::yield_now().await;

        assert!(!child2.is_alive());
        assert_eq!(
            (child0.req(()).await.unwrap(), child1.req(()).await.unwrap()),
            (1, 0)
        );
    }
}

use derive_more::From;
use speare::{req_res, Actor, Ctx, ExitReason, Handle, Node, Request, Supervision};
use std::time::Duration;
use tokio::{task, time};
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

    struct MaxResetAmount {
        child: Handle<ChildMsg>,
    }

    impl Actor for MaxResetAmount {
        type Props = ();
        type Msg = Request<(), Handle<ChildMsg>>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(MaxResetAmount {
                child: ctx
                    .actor::<Child>(0)
                    .supervision(Supervision::Restart {
                        max: 3.into(),
                        backoff: Backoff::None,
                    })
                    .spawn(),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.child.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn reaches_max_reset_limit_and_shuts_down_actor() {
        // Arrange
        let mut node = Node::default();
        let max_reset = node.actor::<MaxResetAmount>(()).spawn();

        let (req, res) = req_res(());
        max_reset.send(req);
        let child = res.recv().await.unwrap();
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

    #[derive(Clone)]
    struct Parent {
        child0: Handle<ChildMsg>,
        child1: Handle<ChildMsg>,
        child2: Handle<ChildMsg>,
    }

    impl Actor for Parent {
        type Props = ();
        type Msg = Request<(), Parent>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(Parent {
                child0: ctx
                    .actor::<Child>(0)
                    .supervision(Supervision::Resume)
                    .spawn(),

                child1: ctx
                    .actor::<Child>(1)
                    .supervision(Supervision::Restart {
                        max: Limit::None,
                        backoff: Backoff::None,
                    })
                    .spawn(),

                child2: ctx.actor::<Child>(2).spawn(),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn all_different_strategies_work() {
        // Arrange
        let mut node = Node::default();
        let root = node.actor::<Parent>(()).spawn();

        let (req, res) = req_res(());
        root.send(req);
        let Parent {
            child0,
            child1,
            child2,
        } = res.recv().await.unwrap();

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
        time::sleep(Duration::from_nanos(1)).await;

        assert!(!child2.is_alive());
        assert_eq!(
            (child0.req(()).await.unwrap(), child1.req(()).await.unwrap()),
            (1, 0)
        );
    }
}

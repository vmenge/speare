use async_trait::async_trait;
use derive_more::From;
use speare::{req_res, Actor, Ctx, Directive, ExitReason, Handle, Node, Request, Supervision};
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

#[async_trait]
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

    async fn exit(&mut self, reason: ExitReason<Self>, _: &mut Ctx<Self>) {
        println!("Child exiting. {:?}", reason);
    }
}

mod one_for_one {
    use super::*;

    struct MaxResetAmount {
        child: Handle<ChildMsg>,
    }

    #[async_trait]
    impl Actor for MaxResetAmount {
        type Props = ();
        type Msg = Request<(), Handle<ChildMsg>>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(MaxResetAmount {
                child: ctx.spawn::<Child>(0),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.child.clone());
            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_one().max_restarts(2)
        }
    }

    #[tokio::test]
    async fn reaches_max_reset_limit_and_shuts_down_actor() {
        // Arrange
        let node = Node::default();
        let max_reset = node.spawn::<MaxResetAmount>(());

        let (req, res) = req_res(());
        max_reset.send(req);
        let child = res.recv().await.unwrap();
        let kill = || async {
            child.send(ChildMsg::Fail);
            time::sleep(Duration::from_nanos(1)).await; // wait for actor to be killed
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

    struct MaxResetWithin {
        child: Handle<ChildMsg>,
    }

    #[async_trait]
    impl Actor for MaxResetWithin {
        type Props = ();
        type Msg = Request<(), Handle<ChildMsg>>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(MaxResetWithin {
                child: ctx.spawn::<Child>(0),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.child.clone());
            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_one().max_restarts((1, Duration::from_secs(1)))
        }
    }

    #[tokio::test]
    async fn shuts_down_actor_only_if_reset_limit_is_reached_within_duration() {
        // Arrange
        let node = Node::default();
        let max_reset = node.spawn::<MaxResetWithin>(());

        let (req, res) = req_res(());
        max_reset.send(req);
        let child = res.recv().await.unwrap();
        let kill = || async {
            child.send(ChildMsg::Fail);
            time::sleep(Duration::from_nanos(1)).await; // wait for actor to be killed
        };

        // Act & Assert

        // No restarts, should be alive
        assert!(child.is_alive());

        // 1st restart, should be alive
        kill().await;
        assert!(child.is_alive());

        time::pause();
        time::advance(Duration::from_secs(10)).await;
        time::resume();

        // 1st restart, Should still be alive as restart counter was reset after timespan passed
        kill().await;
        assert!(child.is_alive());

        // 2nd restart, should be dead as we didn't advance time and restart limit within timespan was reached
        kill().await;
        assert!(!child.is_alive());
    }

    #[derive(Clone)]
    struct Parent {
        child0: Handle<ChildMsg>,
        child1: Handle<ChildMsg>,
        child2: Handle<ChildMsg>,
    }

    #[async_trait]
    impl Actor for Parent {
        type Props = ();
        type Msg = Request<(), Parent>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(Parent {
                child0: ctx.spawn::<Child>(0),
                child1: ctx.spawn::<Child>(1),
                child2: ctx.spawn::<Child>(2),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.clone());
            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_one().when(|e: &u32| match e {
                0 => Directive::Resume,
                1 => Directive::Restart,
                _ => Directive::Stop,
            })
        }
    }

    #[tokio::test]
    async fn one_for_one_only_affects_failing_actor() {
        // Arrange
        let node = Node::default();
        let root = node.spawn::<Parent>(());

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

    struct EscalateRoot {
        errs: Vec<String>,
    }

    #[derive(From)]
    enum EscalateRootMsg {
        Push(String),
        GetErrs(Request<(), Vec<String>>),
    }

    #[async_trait]
    impl Actor for EscalateRoot {
        type Props = ();
        type Msg = EscalateRootMsg;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.spawn::<EscalateParent>(ctx.this().clone());
            Ok(Self { errs: vec![] })
        }

        async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            match msg {
                EscalateRootMsg::Push(err) => self.errs.push(err),
                EscalateRootMsg::GetErrs(req) => req.reply(self.errs.clone()),
            }

            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_one()
                .when(|e: &EscalateChildErr| {
                    e.0.send(EscalateRootMsg::Push("EscalateChildErr".to_string()));
                    Directive::Resume
                })
                .when(|e: &EscalateParentErr| {
                    e.0.send(EscalateRootMsg::Push("EscalateParentErr".to_string()));
                    Directive::Resume
                })
        }
    }

    struct EscalateParent;

    #[derive(From)]
    struct EscalateParentErr(Handle<EscalateRootMsg>);

    #[async_trait]
    impl Actor for EscalateParent {
        type Props = Handle<EscalateRootMsg>;
        type Msg = ();
        type Err = EscalateParentErr;

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.spawn::<EscalateChild>(ctx.props().clone());
            Ok(Self)
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_one().directive(Directive::Escalate)
        }
    }

    struct EscalateChild;

    #[derive(From)]
    struct EscalateChildErr(Handle<EscalateRootMsg>);

    #[async_trait]
    impl Actor for EscalateChild {
        type Props = Handle<EscalateRootMsg>;
        type Msg = ();
        type Err = EscalateChildErr;

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Err(ctx.props().clone().into())
        }
    }

    #[tokio::test]
    async fn escalates_error() {
        // Arrange
        let node = Node::default();

        // Act
        let root = node.spawn::<EscalateRoot>(());
        task::yield_now().await;
        let errors = root.req(()).await.unwrap();

        // Assert
        assert_eq!(errors, vec!["EscalateChildErr".to_string()])
    }
}

mod one_for_all {
    use crate::sync_vec::SyncVec;

    use super::*;

    #[derive(Clone)]
    struct MaxResetAmount {
        child0: Handle<ChildMsg>,
        child1: Handle<ChildMsg>,
    }

    #[async_trait]
    impl Actor for MaxResetAmount {
        type Props = ();
        type Msg = Request<(), Self>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(MaxResetAmount {
                child0: ctx.spawn::<Child>(0),
                child1: ctx.spawn::<Child>(1),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.clone());
            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_all().max_restarts(2)
        }
    }

    #[tokio::test]
    async fn reaches_max_reset_limit_and_shuts_down_actor() {
        // Arrange
        let node = Node::default();
        let max_reset = node.spawn::<MaxResetAmount>(());

        let (req, res) = req_res(());
        max_reset.send(req);
        let MaxResetAmount { child0, child1 } = res.recv().await.unwrap();

        // Act & Assert

        // No restarts, should be alive
        assert!(child0.is_alive());
        assert!(child1.is_alive());

        // 1st restart, should be alive
        child0.send(ChildMsg::Fail);
        time::sleep(Duration::from_nanos(1)).await;
        assert!(child0.is_alive());
        assert!(child1.is_alive());

        // 2nd restart, should be alive
        child1.send(ChildMsg::Fail);
        time::sleep(Duration::from_nanos(1)).await;
        assert!(child0.is_alive());
        assert!(child1.is_alive());

        // 3rd restart, should be dead
        child0.send(ChildMsg::Fail);
        time::sleep(Duration::from_nanos(1)).await;
        assert!(!child0.is_alive());
        assert!(!child1.is_alive());
    }

    #[derive(Clone)]
    struct MaxResetWithin {
        child0: Handle<ChildMsg>,
        child1: Handle<ChildMsg>,
    }

    #[async_trait]
    impl Actor for MaxResetWithin {
        type Props = ();
        type Msg = Request<(), Self>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(MaxResetWithin {
                child0: ctx.spawn::<Child>(0),
                child1: ctx.spawn::<Child>(1),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.clone());
            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_all().max_restarts((1, Duration::from_secs(1)))
        }
    }

    #[tokio::test]
    async fn shuts_down_actor_only_if_reset_limit_is_reached_within_duration() {
        // Arrange
        let node = Node::default();
        let max_reset = node.spawn::<MaxResetWithin>(());

        let (req, res) = req_res(());
        max_reset.send(req);
        let MaxResetWithin { child0, child1 } = res.recv().await.unwrap();

        // Act & Assert

        // No restarts, should be alive
        assert!(child0.is_alive());
        assert!(child1.is_alive());

        // 1st restart, should be alive
        child0.send(ChildMsg::Fail);
        time::sleep(Duration::from_nanos(1)).await;
        assert!(child0.is_alive());

        time::pause();
        time::advance(Duration::from_secs(10)).await;
        time::resume();

        // 1st restart, should still be alive as restart counter was reset after timespan passed
        child1.send(ChildMsg::Fail);
        time::sleep(Duration::from_nanos(1)).await;
        assert!(child0.is_alive());
        assert!(child1.is_alive());

        // 2nd restart, should be dead as we didn't advance time and restart limit within timespan was reached
        child0.send(ChildMsg::Fail);
        time::sleep(Duration::from_nanos(1)).await;
        assert!(!child0.is_alive());
        assert!(!child1.is_alive());
    }

    #[derive(Clone)]
    struct Parent {
        child0: Handle<ChildMsg>,
        child1: Handle<ChildMsg>,
        child2: Handle<ChildMsg>,
    }

    #[async_trait]
    impl Actor for Parent {
        type Props = ();
        type Msg = Request<(), Parent>;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(Parent {
                child0: ctx.spawn::<Child>(0),
                child1: ctx.spawn::<Child>(1),
                child2: ctx.spawn::<Child>(2),
            })
        }

        async fn handle(&mut self, req: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            req.reply(self.clone());
            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_all().when(|e: &u32| match e {
                0 => Directive::Resume,
                1 => Directive::Restart,
                _ => Directive::Stop,
            })
        }
    }

    #[tokio::test]
    async fn one_for_all_affects_all_actors() {
        // Arrange
        let node = Node::default();
        let root = node.spawn::<Parent>(());

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

        // Fail child1, all counts should go back to 0
        child1.send(ChildMsg::Fail);
        task::yield_now().await;

        assert_eq!(counts().await, (0, 0, 0));

        // Fail child2, all Actors should stop
        child2.send(ChildMsg::Fail);
        time::sleep(Duration::from_nanos(1)).await;

        assert!(!child0.is_alive());
        assert!(!child1.is_alive());
        assert!(!child2.is_alive());
    }

    struct EscalateRoot {
        errs: Vec<String>,
    }

    #[derive(From)]
    enum EscalateRootMsg {
        Push(String),
        GetErrs(Request<(), Vec<String>>),
    }

    #[async_trait]
    impl Actor for EscalateRoot {
        type Props = ();
        type Msg = EscalateRootMsg;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.spawn::<EscalateParent>(ctx.this().clone());
            Ok(Self { errs: vec![] })
        }

        async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            match msg {
                EscalateRootMsg::Push(err) => self.errs.push(err),
                EscalateRootMsg::GetErrs(req) => req.reply(self.errs.clone()),
            }

            Ok(())
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_all()
                .when(|e: &EscalateChildErr| {
                    e.0.send(EscalateRootMsg::Push("EscalateChildErr".to_string()));
                    Directive::Resume
                })
                .when(|e: &EscalateParentErr| {
                    e.0.send(EscalateRootMsg::Push("EscalateParentErr".to_string()));
                    Directive::Resume
                })
        }
    }

    struct EscalateParent;

    #[derive(From)]
    struct EscalateParentErr(Handle<EscalateRootMsg>);

    #[async_trait]
    impl Actor for EscalateParent {
        type Props = Handle<EscalateRootMsg>;
        type Msg = ();
        type Err = EscalateParentErr;

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.spawn::<EscalateChild>(ctx.props().clone());
            Ok(Self)
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_one().directive(Directive::Escalate)
        }
    }

    struct EscalateChild;

    #[derive(From)]
    struct EscalateChildErr(Handle<EscalateRootMsg>);

    #[async_trait]
    impl Actor for EscalateChild {
        type Props = Handle<EscalateRootMsg>;
        type Msg = ();
        type Err = EscalateChildErr;

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Err(ctx.props().clone().into())
        }
    }

    #[tokio::test]
    async fn escalates_error() {
        // Arrange
        let node = Node::default();

        // Act
        let root = node.spawn::<EscalateRoot>(());
        task::yield_now().await;
        let errors = root.req(()).await.unwrap();

        // Assert
        assert_eq!(errors, vec!["EscalateChildErr".to_string()])
    }

    struct Dad {
        kid0: Handle<()>,
    }

    #[async_trait]
    impl Actor for Dad {
        type Props = SyncVec<String>;
        type Msg = ();
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.props().push("Dad::init".to_string()).await;
            let kid0 = ctx.spawn::<Kid>((0, ctx.props().clone()));
            ctx.spawn::<Kid>((1, ctx.props().clone()));
            ctx.spawn::<Kid>((2, ctx.props().clone()));

            Ok(Self { kid0 })
        }

        async fn exit(&mut self, _: ExitReason<Self>, ctx: &mut Ctx<Self>) {
            ctx.props().push("Dad::exit".to_string()).await;
        }

        fn supervision(_: &Self::Props) -> Supervision {
            Supervision::one_for_all().directive(Directive::Restart)
        }

        async fn handle(&mut self, _: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            self.kid0.send(());
            Ok(())
        }
    }

    struct Kid;

    #[async_trait]
    impl Actor for Kid {
        type Props = (Id, SyncVec<String>);
        type Msg = ();
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            let (id, evts) = ctx.props();
            evts.push(format!("Kid{id}::init")).await;
            Ok(Self)
        }

        async fn exit(&mut self, _: ExitReason<Self>, ctx: &mut Ctx<Self>) {
            let (id, evts) = ctx.props();
            evts.push(format!("Kid{id}::exit")).await;
        }

        async fn handle(&mut self, _: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            Err(())
        }
    }

    #[tokio::test]
    async fn stops_all_children_before_starting_them_again() {
        // Arrange
        let evts = SyncVec::default();
        let node = Node::default();
        let dad = node.spawn::<Dad>(evts.clone());

        // Act
        dad.send(());
        time::sleep(Duration::from_millis(1)).await;

        // Assert
        let evts = evts.clone_vec().await;
        println!("{:?}", evts);
        assert_eq!(evts.len(), 10, "{:?}", evts);

        let evts_clone = evts.clone();
        let err_msg = move |idx: usize, actual: &str| {
            evts_clone
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    if i == idx {
                        format!("{x} -- ACTUAL VALUE. EXPECTED: {actual}")
                    } else {
                        x.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(",\n")
        };

        evts.iter()
            .enumerate()
            .take(4)
            .for_each(|(i, x)| assert!(x.ends_with("init"), "{}", err_msg(i, x)));

        evts.iter().enumerate().skip(4).take(3).for_each(|(i, x)| {
            assert!(x.starts_with("Kid"), "{}", err_msg(i, "Kid{}::exit"));
            assert!(x.ends_with("exit"), "{}", err_msg(i, "Kid{}::exit"));
        });

        evts.iter().enumerate().skip(7).for_each(|(i, x)| {
            assert!(x.starts_with("Kid"), "{}", err_msg(i, "Kid{}::init"));
            assert!(x.ends_with("init"), "{}", err_msg(i, "Kid{}::init"));
        });
    }
}

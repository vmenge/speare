use async_trait::async_trait;
use derive_more::From;
use speare::{req_res, Ctx, Directive, ExitReason, Handle, Node, Process, Request, Supervision};
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
impl Process for Child {
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
    impl Process for MaxResetAmount {
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
    async fn reaches_max_reset_limit_and_shuts_down_process() {
        // Arrange
        let mut node = Node::default();
        let max_reset = node.spawn::<MaxResetAmount>(());

        let (req, res) = req_res(());
        max_reset.send(req);
        let child = res.recv().await.unwrap();
        let kill = || async {
            child.send(ChildMsg::Fail);
            time::sleep(Duration::from_nanos(1)).await; // wait for process to be killed
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
    impl Process for MaxResetWithin {
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
    async fn shuts_down_process_only_if_reset_limit_is_reached_within_duration() {
        // Arrange
        let mut node = Node::default();
        let max_reset = node.spawn::<MaxResetWithin>(());

        let (req, res) = req_res(());
        max_reset.send(req);
        let child = res.recv().await.unwrap();
        let kill = || async {
            child.send(ChildMsg::Fail);
            time::sleep(Duration::from_nanos(1)).await; // wait for process to be killed
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
    impl Process for Parent {
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
    async fn one_for_one_only_affects_failing_process() {
        // Arrange
        let mut node = Node::default();
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

        // Fail child2, Process should stop. Other counts should remain unaffected.
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
    impl Process for EscalateRoot {
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
    impl Process for EscalateParent {
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
    impl Process for EscalateChild {
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
        let mut node = Node::default();

        // Act
        let root = node.spawn::<EscalateRoot>(());
        task::yield_now().await;
        let errors = root.req(()).await.unwrap();

        // Assert
        assert_eq!(errors, vec!["EscalateChildErr".to_string()])
    }
}

mod one_for_all {
    use super::*;

    #[derive(Clone)]
    struct MaxResetAmount {
        child0: Handle<ChildMsg>,
        child1: Handle<ChildMsg>,
    }

    #[async_trait]
    impl Process for MaxResetAmount {
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
    async fn reaches_max_reset_limit_and_shuts_down_process() {
        // Arrange
        let mut node = Node::default();
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
    impl Process for MaxResetWithin {
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
    async fn shuts_down_process_only_if_reset_limit_is_reached_within_duration() {
        // Arrange
        let mut node = Node::default();
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
    impl Process for Parent {
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
    async fn one_for_all_affects_all_process() {
        // Arrange
        let mut node = Node::default();
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

        // Fail child2, all Processes should stop
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
    impl Process for EscalateRoot {
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
    impl Process for EscalateParent {
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
    impl Process for EscalateChild {
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
        let mut node = Node::default();

        // Act
        let root = node.spawn::<EscalateRoot>(());
        task::yield_now().await;
        let errors = root.req(()).await.unwrap();

        // Assert
        assert_eq!(errors, vec!["EscalateChildErr".to_string()])
    }
}

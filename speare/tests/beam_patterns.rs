mod sync_vec;

use speare::{Actor, Backoff, Ctx, Handle, Limit, Node, Supervision};
use sync_vec::SyncVec;
use tokio::task;

// ---------------------------------------------------------------------------
// Shared child actor: records each init and can be told to fail
// ---------------------------------------------------------------------------

struct Worker;

enum WorkerMsg {
    Fail,
}

#[derive(Debug)]
struct WorkerErr;

struct WorkerProps {
    id: u32,
    inits: SyncVec<u32>,
}

impl Actor for Worker {
    type Props = WorkerProps;
    type Msg = WorkerMsg;
    type Err = WorkerErr;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().inits.push(ctx.props().id).await;
        Ok(Worker)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            WorkerMsg::Fail => Err(WorkerErr),
        }
    }
}

// ===========================================================================
// one_for_all
// ===========================================================================

mod one_for_all {
    use super::*;

    struct Supervisor;

    enum SupervisorMsg {
        ChildFailed,
    }

    struct SupervisorProps {
        inits: SyncVec<u32>,
    }

    impl Actor for Supervisor {
        type Props = SupervisorProps;
        type Msg = SupervisorMsg;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            spawn_all(ctx, ctx.props().inits.clone());
            Ok(Supervisor)
        }

        async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
            match msg {
                SupervisorMsg::ChildFailed => {
                    ctx.stop_children().await;
                    spawn_all(ctx, ctx.props().inits.clone());
                }
            }

            Ok(())
        }
    }

    fn spawn_all(ctx: &mut Ctx<Supervisor>, inits: SyncVec<u32>) {
        ctx.actor::<Worker>(WorkerProps {
            id: 1,
            inits: inits.clone(),
        })
        .watch(|_| SupervisorMsg::ChildFailed)
        .spawn_named("worker-a")
        .unwrap();

        ctx.actor::<Worker>(WorkerProps {
            id: 2,
            inits: inits.clone(),
        })
        .watch(|_| SupervisorMsg::ChildFailed)
        .spawn_named("worker-b")
        .unwrap();

        ctx.actor::<Worker>(WorkerProps {
            id: 3,
            inits: inits.clone(),
        })
        .watch(|_| SupervisorMsg::ChildFailed)
        .spawn_named("worker-c")
        .unwrap();
    }

    fn get_workers(node: &Node) -> (Handle<WorkerMsg>, Handle<WorkerMsg>, Handle<WorkerMsg>) {
        (
            node.get_handle::<WorkerMsg>("worker-a").unwrap(),
            node.get_handle::<WorkerMsg>("worker-b").unwrap(),
            node.get_handle::<WorkerMsg>("worker-c").unwrap(),
        )
    }

    #[tokio::test]
    async fn failing_one_child_restarts_all_children() {
        // Arrange
        let mut node = Node::default();
        let inits = SyncVec::<u32>::default();
        node.actor::<Supervisor>(SupervisorProps {
            inits: inits.clone(),
        })
        .spawn();
        task::yield_now().await;
        assert_eq!(inits.clone_vec().await, vec![1, 2, 3]);

        // Act -- fail worker A (Supervision::Stop -> watch fires -> ChildFailed)
        let (a, _, _) = get_workers(&node);
        a.send(WorkerMsg::Fail);
        task::yield_now().await;

        // Assert -- all three workers were re-spawned (new init calls)
        assert_eq!(inits.clone_vec().await, vec![1, 2, 3, 1, 2, 3]);
    }

    #[tokio::test]
    async fn old_handles_become_dead_after_group_restart() {
        // Arrange
        let mut node = Node::default();
        let inits = SyncVec::<u32>::default();
        node.actor::<Supervisor>(SupervisorProps {
            inits: inits.clone(),
        })
        .spawn();
        task::yield_now().await;

        let (a, b, c) = get_workers(&node);

        // Act -- trigger one_for_all
        a.send(WorkerMsg::Fail);
        task::yield_now().await;

        // Assert -- the old handles should be dead since
        // stop_children was called and new children were spawned
        assert!(!a.is_alive());
        assert!(!b.is_alive());
        assert!(!c.is_alive());
    }

    #[tokio::test]
    async fn new_handles_are_alive_after_group_restart() {
        // Arrange
        let mut node = Node::default();
        let inits = SyncVec::<u32>::default();
        node.actor::<Supervisor>(SupervisorProps {
            inits: inits.clone(),
        })
        .spawn();
        task::yield_now().await;

        let (a, _, _) = get_workers(&node);

        // Act -- trigger one_for_all
        a.send(WorkerMsg::Fail);
        task::yield_now().await;

        // Assert -- new handles from registry are alive
        let (a2, b2, c2) = get_workers(&node);
        assert!(a2.is_alive());
        assert!(b2.is_alive());
        assert!(c2.is_alive());
    }

    #[tokio::test]
    async fn failing_any_child_triggers_group_restart() {
        // Arrange
        let mut node = Node::default();
        let inits = SyncVec::<u32>::default();
        node.actor::<Supervisor>(SupervisorProps {
            inits: inits.clone(),
        })
        .spawn();
        task::yield_now().await;

        // Act -- fail worker B
        let (_, b, _) = get_workers(&node);
        b.send(WorkerMsg::Fail);
        task::yield_now().await;

        // Assert -- all three re-spawned
        assert_eq!(inits.clone_vec().await, vec![1, 2, 3, 1, 2, 3]);
    }

    #[tokio::test]
    async fn with_individual_restarts_before_escalation() {
        // Each child gets 2 individual restarts before watch fires
        struct Sup2;

        enum Sup2Msg {
            ChildFailed,
        }

        struct Sup2Props {
            inits: SyncVec<u32>,
            watch_fires: SyncVec<()>,
        }

        fn spawn_all(ctx: &mut Ctx<Sup2>, inits: SyncVec<u32>) {
            ctx.actor::<Worker>(WorkerProps {
                id: 1,
                inits: inits.clone(),
            })
            .supervision(Supervision::Restart {
                max: Limit::Amount(2),
                backoff: Backoff::None,
            })
            .watch(|_| Sup2Msg::ChildFailed)
            .spawn_named("worker-a")
            .unwrap();

            ctx.actor::<Worker>(WorkerProps {
                id: 2,
                inits: inits.clone(),
            })
            .supervision(Supervision::Restart {
                max: Limit::Amount(2),
                backoff: Backoff::None,
            })
            .watch(|_| Sup2Msg::ChildFailed)
            .spawn_named("worker-b")
            .unwrap();
        }

        impl Actor for Sup2 {
            type Props = Sup2Props;
            type Msg = Sup2Msg;
            type Err = ();

            async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
                spawn_all(ctx, ctx.props().inits.clone());
                Ok(Sup2)
            }

            async fn handle(
                &mut self,
                msg: Self::Msg,
                ctx: &mut Ctx<Self>,
            ) -> Result<(), Self::Err> {
                match msg {
                    Sup2Msg::ChildFailed => {
                        ctx.props().watch_fires.push(()).await;
                        ctx.stop_children().await;
                        spawn_all(ctx, ctx.props().inits.clone());
                    }
                }

                Ok(())
            }
        }

        // Arrange
        let mut node = Node::default();
        let inits = SyncVec::<u32>::default();
        let watch_fires = SyncVec::<()>::default();
        node.actor::<Sup2>(Sup2Props {
            inits: inits.clone(),
            watch_fires: watch_fires.clone(),
        })
        .spawn();
        task::yield_now().await;
        assert_eq!(inits.clone_vec().await, vec![1, 2]);

        // Act -- fail A once -> individual restart, no group restart
        let a = node.get_handle::<WorkerMsg>("worker-a").unwrap();
        a.send(WorkerMsg::Fail);
        task::yield_now().await;
        assert!(
            watch_fires.clone_vec().await.is_empty(),
            "watch should not fire after 1st fail"
        );
        // A got individually restarted
        assert_eq!(inits.clone_vec().await, vec![1, 2, 1]);

        // Act -- fail A again -> exhausts 2 restarts, watch fires -> group restart
        a.send(WorkerMsg::Fail);
        task::yield_now().await;
        assert_eq!(
            watch_fires.clone_vec().await.len(),
            1,
            "watch should fire after max restarts"
        );

        // Both workers were re-spawned as part of group restart
        assert_eq!(inits.clone_vec().await, vec![1, 2, 1, 1, 2]);
    }
}

// ===========================================================================
// rest_for_one
// ===========================================================================

mod rest_for_one {
    use super::*;

    // Option 1: stop_children + re-spawn everything
    mod option1_clean_slate {
        use super::*;

        struct Supervisor;

        enum SupervisorMsg {
            WorkerAFailed,
        }

        struct SupervisorProps {
            inits: SyncVec<u32>,
        }

        impl Actor for Supervisor {
            type Props = SupervisorProps;
            type Msg = SupervisorMsg;
            type Err = ();

            async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
                spawn_all(ctx, ctx.props().inits.clone());
                Ok(Supervisor)
            }

            async fn handle(
                &mut self,
                msg: Self::Msg,
                ctx: &mut Ctx<Self>,
            ) -> Result<(), Self::Err> {
                match msg {
                    SupervisorMsg::WorkerAFailed => {
                        ctx.stop_children().await;
                        spawn_all(ctx, ctx.props().inits.clone());
                    }
                }

                Ok(())
            }
        }

        fn spawn_all(ctx: &mut Ctx<Supervisor>, inits: SyncVec<u32>) {
            // Only worker A is watched -- it's the foundational actor
            ctx.actor::<Worker>(WorkerProps {
                id: 1,
                inits: inits.clone(),
            })
            .watch(|_| SupervisorMsg::WorkerAFailed)
            .spawn_named("worker-a")
            .unwrap();

            ctx.actor::<Worker>(WorkerProps {
                id: 2,
                inits: inits.clone(),
            })
            .spawn_named("worker-b")
            .unwrap();

            ctx.actor::<Worker>(WorkerProps {
                id: 3,
                inits: inits.clone(),
            })
            .spawn_named("worker-c")
            .unwrap();
        }

        fn get_workers(node: &Node) -> (Handle<WorkerMsg>, Handle<WorkerMsg>, Handle<WorkerMsg>) {
            (
                node.get_handle::<WorkerMsg>("worker-a").unwrap(),
                node.get_handle::<WorkerMsg>("worker-b").unwrap(),
                node.get_handle::<WorkerMsg>("worker-c").unwrap(),
            )
        }

        #[tokio::test]
        async fn foundational_child_failure_restarts_all() {
            // Arrange
            let mut node = Node::default();
            let inits = SyncVec::<u32>::default();
            node.actor::<Supervisor>(SupervisorProps {
                inits: inits.clone(),
            })
            .spawn();
            task::yield_now().await;
            assert_eq!(inits.clone_vec().await, vec![1, 2, 3]);

            // Act -- fail foundational worker A
            let (a, _, _) = get_workers(&node);
            a.send(WorkerMsg::Fail);
            task::yield_now().await;

            // Assert -- all workers re-spawned
            assert_eq!(inits.clone_vec().await, vec![1, 2, 3, 1, 2, 3]);
        }

        #[tokio::test]
        async fn non_foundational_child_failure_does_not_trigger_group_restart() {
            // Arrange
            let mut node = Node::default();
            let inits = SyncVec::<u32>::default();
            node.actor::<Supervisor>(SupervisorProps {
                inits: inits.clone(),
            })
            .spawn();
            task::yield_now().await;
            assert_eq!(inits.clone_vec().await, vec![1, 2, 3]);

            // Act -- fail non-foundational worker B (no watch on B)
            let (_, b, _) = get_workers(&node);
            b.send(WorkerMsg::Fail);
            task::yield_now().await;

            // Assert -- B is dead, but no group restart happened
            assert!(!b.is_alive());
            assert_eq!(inits.clone_vec().await, vec![1, 2, 3]);
        }

        #[tokio::test]
        async fn old_handles_dead_after_foundational_failure() {
            // Arrange
            let mut node = Node::default();
            let inits = SyncVec::<u32>::default();
            node.actor::<Supervisor>(SupervisorProps {
                inits: inits.clone(),
            })
            .spawn();
            task::yield_now().await;

            let (a, b, c) = get_workers(&node);

            // Act -- fail foundational
            a.send(WorkerMsg::Fail);
            task::yield_now().await;

            // All old handles dead
            assert!(!a.is_alive());
            assert!(!b.is_alive());
            assert!(!c.is_alive());

            // New handles are alive
            let (a2, b2, c2) = get_workers(&node);
            assert!(a2.is_alive());
            assert!(b2.is_alive());
            assert!(c2.is_alive());
        }
    }

    // Option 2: restart_children + re-spawn the dead actor
    mod option2_preserve_mailbox {
        use super::*;

        struct Supervisor;

        enum SupervisorMsg {
            WorkerAFailed,
        }

        struct SupervisorProps {
            inits: SyncVec<u32>,
        }

        impl Actor for Supervisor {
            type Props = SupervisorProps;
            type Msg = SupervisorMsg;
            type Err = ();

            async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
                let inits = ctx.props().inits.clone();

                ctx.actor::<Worker>(WorkerProps {
                    id: 1,
                    inits: inits.clone(),
                })
                .watch(|_| SupervisorMsg::WorkerAFailed)
                .spawn_named("worker-a")
                .unwrap();

                ctx.actor::<Worker>(WorkerProps {
                    id: 2,
                    inits: inits.clone(),
                })
                .supervision(Supervision::Restart {
                    max: Limit::None,
                    backoff: Backoff::None,
                })
                .spawn();

                ctx.actor::<Worker>(WorkerProps {
                    id: 3,
                    inits: inits.clone(),
                })
                .supervision(Supervision::Restart {
                    max: Limit::None,
                    backoff: Backoff::None,
                })
                .spawn();

                Ok(Supervisor)
            }

            async fn handle(
                &mut self,
                msg: Self::Msg,
                ctx: &mut Ctx<Self>,
            ) -> Result<(), Self::Err> {
                match msg {
                    SupervisorMsg::WorkerAFailed => {
                        // restart_children restarts surviving B and C (preserving mailbox)
                        ctx.restart_children();
                        // re-spawn the dead A
                        let inits = ctx.props().inits.clone();
                        ctx.actor::<Worker>(WorkerProps {
                            id: 1,
                            inits: inits.clone(),
                        })
                        .watch(|_| SupervisorMsg::WorkerAFailed)
                        .spawn_named("worker-a")
                        .unwrap();
                    }
                }

                Ok(())
            }
        }

        #[tokio::test]
        async fn foundational_failure_restarts_surviving_and_respawns_dead() {
            // Arrange
            let mut node = Node::default();
            let inits = SyncVec::<u32>::default();
            node.actor::<Supervisor>(SupervisorProps {
                inits: inits.clone(),
            })
            .spawn();
            task::yield_now().await;
            assert_eq!(inits.clone_vec().await, vec![1, 2, 3]);

            // Act -- fail the foundational actor
            let a = node.get_handle::<WorkerMsg>("worker-a").unwrap();
            a.send(WorkerMsg::Fail);
            task::yield_now().await;

            // Assert -- B and C got restart_children (re-init), A was re-spawned
            let all_inits = inits.clone_vec().await;
            // B(2) and C(3) are restarted (order may vary), plus new A(1)
            assert_eq!(all_inits.len(), 6);
            assert_eq!(&all_inits[..3], &[1, 2, 3]);
            let mut restart_set: Vec<u32> = all_inits[3..].to_vec();
            restart_set.sort();
            assert_eq!(restart_set, vec![1, 2, 3]);
        }

        #[tokio::test]
        async fn new_worker_a_handle_is_alive_after_respawn() {
            // Arrange
            let mut node = Node::default();
            let inits = SyncVec::<u32>::default();
            node.actor::<Supervisor>(SupervisorProps {
                inits: inits.clone(),
            })
            .spawn();
            task::yield_now().await;

            // Act -- fail A
            let a = node.get_handle::<WorkerMsg>("worker-a").unwrap();
            a.send(WorkerMsg::Fail);
            task::yield_now().await;

            // Assert
            assert!(!a.is_alive());
            let a2 = node.get_handle::<WorkerMsg>("worker-a").unwrap();
            assert!(a2.is_alive());
        }
    }
}

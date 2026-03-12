mod sync_vec;

use speare::{Actor, Ctx, Node};
use std::time::Duration;
use sync_vec::SyncVec;
use tokio::time;

#[derive(Debug, PartialEq, Clone)]
enum Msg {
    FromTask(u32),
    FromHandle(u32),
}

#[tokio::test]
async fn task_result_is_handled_as_message() {
    // Arrange
    struct Spawner;

    impl Actor for Spawner {
        type Props = SyncVec<Msg>;
        type Msg = Msg;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.task(async { Ok(Msg::FromTask(42)) });
            Ok(Spawner)
        }

        async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
            ctx.props().push(msg).await;
            Ok(())
        }
    }

    let mut node = Node::default();
    let recvd = SyncVec::<Msg>::default();
    let _handle = node.actor::<Spawner>(recvd.clone()).spawn();

    // Act
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![Msg::FromTask(42)]);
}

#[tokio::test]
async fn task_spawned_in_handle() {
    // Arrange
    #[derive(Debug, PartialEq, Clone)]
    enum HMsg {
        SpawnTask,
        FromTask(u32),
    }

    struct HandleSpawner;

    impl Actor for HandleSpawner {
        type Props = SyncVec<HMsg>;
        type Msg = HMsg;
        type Err = ();

        async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            Ok(HandleSpawner)
        }

        async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
            match msg {
                HMsg::SpawnTask => {
                    ctx.task(async {
                        time::sleep(Duration::from_millis(5)).await;
                        Ok(HMsg::FromTask(99))
                    });
                }

                other => {
                    ctx.props().push(other).await;
                }
            }

            Ok(())
        }
    }

    let mut node = Node::default();
    let recvd: SyncVec<HMsg> = Default::default();
    let handle = node.actor::<HandleSpawner>(recvd.clone()).spawn();

    // Act
    handle.send(HMsg::SpawnTask);
    time::sleep(Duration::from_millis(20)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![HMsg::FromTask(99)]);
}

#[tokio::test]
async fn multiple_tasks_all_deliver() {
    // Arrange
    struct MultiTasker;

    impl Actor for MultiTasker {
        type Props = SyncVec<Msg>;
        type Msg = Msg;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.task(async {
                time::sleep(Duration::from_millis(5)).await;
                Ok(Msg::FromTask(1))
            });

            ctx.task(async {
                time::sleep(Duration::from_millis(10)).await;
                Ok(Msg::FromTask(2))
            });

            ctx.task(async {
                time::sleep(Duration::from_millis(15)).await;
                Ok(Msg::FromTask(3))
            });

            Ok(MultiTasker)
        }

        async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
            ctx.props().push(msg).await;
            Ok(())
        }
    }

    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let _handle = node.actor::<MultiTasker>(recvd.clone()).spawn();

    // Act
    time::sleep(Duration::from_millis(30)).await;

    // Assert
    let msgs = recvd.clone_vec().await;
    assert_eq!(msgs.len(), 3);
    assert!(msgs.contains(&Msg::FromTask(1)));
    assert!(msgs.contains(&Msg::FromTask(2)));
    assert!(msgs.contains(&Msg::FromTask(3)));
}

#[tokio::test]
async fn tasks_aborted_when_actor_stops() {
    // Arrange
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let completed = Arc::new(AtomicBool::new(false));
    let completed_clone = completed.clone();

    struct LongTasker;

    impl Actor for LongTasker {
        type Props = Arc<AtomicBool>;
        type Msg = ();
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            let completed = ctx.props().clone();

            ctx.task(async move {
                time::sleep(Duration::from_secs(10)).await;
                completed.store(true, Ordering::SeqCst);
                Ok(())
            });

            Ok(LongTasker)
        }

        async fn handle(&mut self, _: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
            Ok(())
        }
    }

    let mut node = Node::default();
    let handle = node.actor::<LongTasker>(completed_clone).spawn();

    // Act
    time::sleep(Duration::from_millis(10)).await;
    handle.stop();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert!(!handle.is_alive());
    assert!(!completed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn task_and_handle_messages_interleave() {
    // Arrange
    struct Interleaver;

    impl Actor for Interleaver {
        type Props = SyncVec<Msg>;
        type Msg = Msg;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            ctx.task(async {
                time::sleep(Duration::from_millis(15)).await;
                Ok(Msg::FromTask(1))
            });

            Ok(Interleaver)
        }

        async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
            ctx.props().push(msg).await;
            Ok(())
        }
    }

    let mut node = Node::default();
    let recvd: SyncVec<Msg> = Default::default();
    let handle = node.actor::<Interleaver>(recvd.clone()).spawn();

    // Act
    handle.send(Msg::FromHandle(1));
    time::sleep(Duration::from_millis(30)).await;

    // Assert
    let msgs = recvd.clone_vec().await;
    assert_eq!(msgs.len(), 2);
    assert!(msgs.contains(&Msg::FromHandle(1)));
    assert!(msgs.contains(&Msg::FromTask(1)));
}

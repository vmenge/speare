use flume::Receiver;
use speare::{Actor, Backoff, Ctx, Directive, ExitReason, Node, Supervision};
use std::time::Duration;
use sync_vec::SyncVec;
use tokio::time;

mod sync_vec;

struct Root;

enum Msg {
    TaskOk(String),
    TaskErr(String),
}

impl Actor for Root {
    type Props = (SyncVec<String>, SyncVec<String>);
    type Msg = Msg;
    type Err = String;

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Root)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        use Msg::*;
        match msg {
            TaskOk(val) => {
                let oks = ctx.props().0.clone();
                ctx.subtask(async move {
                    oks.push(val).await;
                    Ok(())
                });
            }

            TaskErr(val) => {
                ctx.subtask(async move { Err(val) });
            }
        }

        Ok(())
    }

    async fn exit(_: Option<Self>, reason: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        println!("QUITTING!!! exitreason: {reason:?}");
        if let ExitReason::Err(e) = reason {
            ctx.props().1.push((*e).clone()).await;
        }
    }
}

#[tokio::test]
async fn executes_subtasks() {
    // Arrange
    let mut node = Node::with_supervision(Supervision::one_for_one().directive(Directive::Stop));
    let (oks, errs) = (SyncVec::default(), SyncVec::default());
    let root = node.spawn::<Root>((oks.clone(), errs.clone()));

    // Act
    use Msg::*;
    root.send(TaskOk("foo".to_string()));
    root.send(TaskOk("bar".to_string()));
    root.send(TaskErr("baz".to_string()));
    time::sleep(Duration::from_millis(500)).await;

    // Assert
    assert_eq!(oks.clone_vec().await, vec!["foo", "bar"]);
    assert_eq!(errs.clone_vec().await, vec!["baz"]);
    assert!(!root.is_alive())
}

struct Restarter;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Event {
    Start,
    Stop,
    Restart,
}

impl Actor for Restarter {
    type Props = (Receiver<Event>, SyncVec<Event>);
    type Msg = ();
    type Err = Event;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let (restart, vec) = ctx.props().clone();
        vec.push(Event::Start).await;
        ctx.subtask(async move {
            if let Ok(msg) = restart.recv_async().await {
                return Err(msg);
            }

            Ok(())
        });

        Ok(Restarter)
    }
}

struct RestartRoot;

impl Actor for RestartRoot {
    type Props = (Receiver<Event>, SyncVec<Event>);
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.spawn::<Restarter>(ctx.props().clone());

        Ok(RestartRoot)
    }

    fn supervision(_: &Self::Props) -> Supervision {
        Supervision::one_for_one()
            .backoff(Backoff::Static(Duration::from_millis(50)))
            .when(|e: &Event| match e {
                Event::Restart => Directive::Restart,
                _ => Directive::Stop,
            })
    }
}

#[tokio::test]
async fn subtasks_trigger_proper_supervision() {
    // Arrange
    let mut node = Node::default();
    let (tx, rx) = flume::unbounded();
    let vec = SyncVec::default();
    node.spawn::<RestartRoot>((rx, vec.clone()));

    // Act
    tx.send(Event::Restart).unwrap();
    tx.send(Event::Restart).unwrap();
    tx.send(Event::Stop).unwrap();

    // Assert
    time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        vec.clone_vec().await,
        vec![Event::Start, Event::Start, Event::Start,]
    );
}

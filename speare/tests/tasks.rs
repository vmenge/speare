use async_trait::async_trait;
use speare::{Actor, Ctx, ExitReason, Node};
use std::time::Duration;
use sync_vec::SyncVec;
use tokio::time;

mod sync_vec;

struct Root;

enum Msg {
    TaskOk(String),
    TaskErr(String),
}

#[async_trait]
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

    async fn exit(&mut self, reason: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        if let ExitReason::Err(e) = reason {
            ctx.props().1.push((*e).clone()).await;
        }
    }
}

#[tokio::test]
async fn executes_subtasks() {
    // Arrange
    let node = Node::default();
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

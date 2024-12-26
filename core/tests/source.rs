use async_trait::async_trait;
use speare::*;
use std::{time::Duration, vec};
use sync_vec::SyncVec;
use tokio::time::{self};

mod sync_vec;

struct Source {
    vec: vec::IntoIter<i32>,
}

#[async_trait]
impl Process for Source {
    type Props = SyncVec<i32>;
    type Msg = i32;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Source {
            vec: vec![1, 2, 3].into_iter(),
        })
    }

    async fn source(&mut self) -> Result<Self::Msg, Self::Err> {
        if let Some(i) = self.vec.next() {
            Ok(i)
        } else {
            Err(())
        }
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().push(msg).await;

        Ok(())
    }
}

struct Parent;

#[async_trait]
impl Process for Parent {
    type Props = SyncVec<i32>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.spawn::<Source>(ctx.props().clone());
        Ok(Parent)
    }

    fn supervision(_: &Self::Props) -> Supervision {
        Supervision::one_for_one().directive(Directive::Stop)
    }
}

#[tokio::test]
async fn exits_when_done_if_root_process() {
    let vec = SyncVec::default();
    let node = Node::default();
    node.spawn::<Source>(vec.clone());

    time::sleep(Duration::from_millis(50)).await;

    assert_eq!(vec.clone_vec().await, vec![1, 2, 3])
}

#[tokio::test]
async fn exits_when_done_if_child_process() {
    let vec = SyncVec::default();
    let node = Node::default();
    node.spawn::<Parent>(vec.clone());

    time::sleep(Duration::from_millis(50)).await;

    assert_eq!(vec.clone_vec().await, vec![1, 2, 3])
}

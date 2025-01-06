mod sync_vec;
use speare::{Actor, Ctx, Node};
use sync_vec::SyncVec;
use tokio::task;

#[derive(Debug, PartialEq, Clone, Copy)]
enum TestMsg {
    Foo,
    Bar,
}

struct Foo;

impl Actor for Foo {
    type Props = SyncVec<TestMsg>;
    type Msg = TestMsg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Foo)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().push(msg).await;
        Ok(())
    }
}

struct Bar;

impl Actor for Bar {
    type Props = SyncVec<TestMsg>;
    type Msg = TestMsg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Bar)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().push(msg).await;
        Ok(())
    }
}

#[allow(clippy::disallowed_names)]
#[tokio::test]
async fn sends_msgs_in_correct_order() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<_> = Default::default();
    let foo = node.spawn::<Foo>(recvd.clone());
    let bar = node.spawn::<Bar>(recvd.clone());

    // Act
    foo.send(TestMsg::Foo);
    bar.send(TestMsg::Bar);
    task::yield_now().await;

    // Assert
    assert_eq!(vec![TestMsg::Foo, TestMsg::Bar], recvd.clone_vec().await)
}

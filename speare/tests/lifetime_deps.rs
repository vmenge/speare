mod sync_vec;
use async_trait::async_trait;
use speare::{Actor, Ctx, ExitReason, Node};
use sync_vec::SyncVec;
use tokio::task;

#[derive(Debug, PartialEq, Clone, Copy)]
enum TestMsg {
    FooStarted,
    FooQuit,
    BarStarted,
    BarQuit,
    Child1Started,
    Child1Quit,
    Child2Started,
    Child2Quit,
}

struct Foo;

#[async_trait]
impl Actor for Foo {
    type Props = SyncVec<TestMsg>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().push(TestMsg::FooStarted).await;

        Ok(Foo)
    }

    async fn exit(&mut self, _: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        ctx.props().push(TestMsg::FooQuit).await;
    }
}

type FailOnStart = bool;

struct Bar;

#[async_trait]
impl Actor for Bar {
    type Props = (SyncVec<TestMsg>, FailOnStart);
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().0.push(TestMsg::BarStarted).await;
        ctx.spawn::<Child1>(ctx.props().0.clone());

        if ctx.props().1 {
            Err(())
        } else {
            Ok(Bar)
        }
    }

    async fn exit(&mut self, _: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        ctx.props().0.push(TestMsg::BarQuit).await;
    }
}

struct Child1;

#[async_trait]
impl Actor for Child1 {
    type Props = SyncVec<TestMsg>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().push(TestMsg::Child1Started).await;
        ctx.spawn::<Child2>(ctx.props().clone());
        Ok(Child1)
    }

    async fn exit(&mut self, _: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        ctx.props().push(TestMsg::Child1Quit).await;
    }
}

struct Child2;

#[async_trait]
impl Actor for Child2 {
    type Props = SyncVec<TestMsg>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.props().push(TestMsg::Child2Started).await;
        Ok(Child2)
    }

    async fn exit(&mut self, _: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        ctx.props().push(TestMsg::Child2Quit).await;
    }
}

#[allow(clippy::disallowed_names)]
#[tokio::test]
async fn on_init_and_on_exit_are_called_in_order() {
    // Arrange
    let node = Node::default();
    let recvd: SyncVec<_> = Default::default();
    node.spawn::<Foo>(recvd.clone());
    let fail_to_start = false;
    node.spawn::<Bar>((recvd.clone(), fail_to_start));
    task::yield_now().await;

    // Act
    drop(node);
    task::yield_now().await;
    task::yield_now().await;

    // Assert
    assert_eq!(
        vec![
            TestMsg::FooStarted,
            TestMsg::BarStarted,
            TestMsg::Child1Started,
            TestMsg::Child2Started,
            TestMsg::Child2Quit,
            TestMsg::Child1Quit,
            TestMsg::BarQuit,
            TestMsg::FooQuit,
        ],
        recvd.clone_vec().await
    )
}

#[allow(clippy::disallowed_names)]
#[tokio::test]
async fn order_preserved_even_with_startup_failure() {
    // Arrange
    let mut node = Node::default();
    let recvd: SyncVec<_> = Default::default();
    node.spawn::<Foo>(recvd.clone());
    let fail_to_start = true;
    node.spawn::<Bar>((recvd.clone(), fail_to_start));
    task::yield_now().await;

    // Act
    drop(node);
    task::yield_now().await;
    task::yield_now().await;

    // Assert
    assert_eq!(
        vec![
            TestMsg::FooStarted,
            TestMsg::BarStarted,
            TestMsg::Child1Started,
            TestMsg::Child2Started,
            TestMsg::Child2Quit,
            TestMsg::Child1Quit,
            TestMsg::FooQuit,
            // No TestMsg::BarQuit because Foo failed to even start
        ],
        recvd.clone_vec().await
    )
}

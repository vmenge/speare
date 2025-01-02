use async_trait::async_trait;
use speare::{req_res, Actor, Ctx, Directive, Handle, Node, Request, Supervision};
use std::time::Duration;
use tokio::time;

struct Foo;

struct FooErr(Handle<ParentMsg>);

#[async_trait]
impl Actor for Foo {
    type Props = Handle<ParentMsg>;
    type Msg = ();
    type Err = FooErr;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Err(FooErr(ctx.props().clone()))
    }
}

struct Bar;

struct BarErr(Handle<ParentMsg>);

#[async_trait]
impl Actor for Bar {
    type Props = Handle<ParentMsg>;
    type Msg = ();
    type Err = BarErr;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Err(BarErr(ctx.props().clone()))
    }
}

struct Parent {
    msgs: Vec<String>,
}

enum ParentMsg {
    Error(String),
    GetMsgs(Request<(), Vec<String>>),
}

#[async_trait]
impl Actor for Parent {
    type Props = ();
    type Msg = ParentMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.spawn::<Foo>(ctx.this().clone());
        ctx.spawn::<Bar>(ctx.this().clone());

        Ok(Parent { msgs: vec![] })
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ParentMsg::Error(err) => self.msgs.push(err),
            ParentMsg::GetMsgs(req) => req.reply(self.msgs.clone()),
        }

        Ok(())
    }

    fn supervision(_: &Self::Props) -> Supervision {
        Supervision::one_for_one()
            .max_restarts(2)
            .when(|e: &FooErr| {
                e.0.send(ParentMsg::Error("Foo".to_string()));
                Directive::Resume
            })
            .when(|e: &BarErr| {
                e.0.send(ParentMsg::Error("Bar".to_string()));
                Directive::Restart
            })
    }
}

#[tokio::test]
async fn it_actor_different_errors_differently() {
    // Arrange
    let mut node = Node::default();
    let parent = node.spawn::<Parent>(());
    time::sleep(Duration::from_nanos(1)).await;

    // Act
    let (req, res) = req_res(());
    parent.send(ParentMsg::GetMsgs(req));
    let msgs = res.recv().await.unwrap();

    // Assert
    assert_eq!(
        msgs,
        vec![
            "Foo".to_string(),
            "Bar".to_string(),
            "Bar".to_string(),
            "Bar".to_string()
        ]
    );
}

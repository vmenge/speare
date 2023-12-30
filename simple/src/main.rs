use std::time::{Duration, Instant};

use async_trait::async_trait;
use simple::{ask, Backoff, Ctx, Directive, Handle, Node, Process, Request, Strategy, Supervision};
use tokio::{task, time};

struct Parent {
    child1: Handle<Child1Msg>,
    child2: Handle<Child2Msg>,
}

enum ParentMsg {
    GetChild1Handle(Request<(), Handle<Child1Msg>>),
}

#[async_trait]
impl Process for Parent {
    type Props = ();
    type Msg = ParentMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, ()> {
        println!("starting PARENT");
        Ok(Self {
            child1: ctx.spawn::<Child1>(()),
            child2: ctx.spawn::<Child2>(()),
        })
    }

    async fn handle(&mut self, msg: ParentMsg, _: &mut Ctx<Self>) -> Result<(), ()> {
        match msg {
            ParentMsg::GetChild1Handle(req) => req.reply(self.child1.clone()),
        }

        Ok(())
    }

    async fn exit(&mut self, _: &mut Ctx<Self>) {
        println!("exiting PARENT");
    }

    fn supervision() -> Supervision {
        Supervision::one_for_all()
            .directive(Directive::Restart {
                max: None,
                backoff: Backoff::None,
            })
            .when(|_: &String| {
                println!("got string");
                Directive::Restart {
                    max: None,
                    backoff: Backoff::None,
                }
            })
            .when(|_: &u8| {
                println!("got u8!");
                Directive::Resume
            })
    }
}

#[derive(Debug)]
struct Child1 {
    count: u8,
}

enum Child1Msg {
    SayHi,
}

#[async_trait]
impl Process for Child1 {
    type Props = ();
    type Msg = Child1Msg;
    type Err = String;

    async fn init(_: &mut Ctx<Self>) -> Result<Self, String> {
        println!("starting CHILD1");
        Ok(Child1 { count: 0 })
    }

    async fn handle(&mut self, _: Child1Msg, _: &mut Ctx<Self>) -> Result<(), String> {
        self.count += 1;
        println!("{:?}: Hi!!!", self);

        if self.count > 1 {
            return Err("boop".to_string());
        }

        Ok(())
    }

    async fn exit(&mut self, ctx: &mut Ctx<Self>) {
        println!("exiting CHILD1");
    }
}

struct Child2;

enum Child2Msg {
    SayHo,
}

#[async_trait]
impl Process for Child2 {
    type Props = ();
    type Msg = Child2Msg;
    type Err = u8;

    async fn init(_: &mut Ctx<Self>) -> Result<Self, u8> {
        println!("starting CHILD2");
        Err(0)
    }

    async fn exit(&mut self, ctx: &mut Ctx<Self>) {
        println!("exiting CHILD2");
    }
}

struct Sibling;

#[async_trait]
impl Process for Sibling {
    type Props = Handle<Child1Msg>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, ()> {
        println!("starting SIBLING");
        Ok(Sibling)
    }

    async fn handle(&mut self, _: (), ctx: &mut Ctx<Self>) -> Result<(), ()> {
        ctx.props().send(Child1Msg::SayHi);
        Ok(())
    }

    async fn exit(&mut self, _: &mut Ctx<Self>) {
        println!("exiting SIBLING");
    }
}

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    let parent = node.spawn::<Parent>(());

    let (req, res) = ask(());
    parent.send(ParentMsg::GetChild1Handle(req));
    let child1 = res.recv().await.unwrap();

    let sibling = node.spawn::<Sibling>(child1);
    sibling.send(());
    sibling.send(());
    sibling.send(());
    sibling.send(());

    time::sleep(Duration::from_secs(1)).await;
    drop(node);
    time::sleep(Duration::from_secs(1)).await;
}

use speare::{req_res, Actor, Ctx, Directive, Handle, Node, Request, Supervision};
use tokio::task;

struct Foo;

impl Actor for Foo {
    type Props = ();
    type Msg = ();
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Foo)
    }
}

struct Bar;

impl Actor for Bar {
    type Props = ();
    type Msg = ();
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Bar)
    }
}

#[allow(clippy::disallowed_names)]
#[tokio::test]
async fn node_stops_all_actors_when_dropped() {
    // Arrange
    let mut node = Node::default();
    let foo = node.spawn::<Foo>(());
    let bar = node.spawn::<Bar>(());

    // Act
    drop(node);
    task::yield_now().await;
    task::yield_now().await;

    // Assert
    assert!(!foo.is_alive());
    assert!(!bar.is_alive());
}

struct Quitter;

impl Actor for Quitter {
    type Props = bool;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        if *ctx.props() {
            Err(())
        } else {
            Ok(Quitter)
        }
    }

    async fn handle(&mut self, _: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        Err(())
    }
}

#[tokio::test]
async fn root_supervision_works() {
    // Arrange
    let mut node = Node::with_supervision(Supervision::one_for_one().directive(Directive::Stop));
    let quit_on_start = false;
    let quitter = node.spawn::<Quitter>(quit_on_start);
    assert!(quitter.is_alive());

    // Error on handle
    // Act
    quitter.send(());
    task::yield_now().await;
    task::yield_now().await;

    // Assert
    assert!(!quitter.is_alive());

    // Error on init
    // Act
    let quit_on_start = true;
    let quitter2 = node.spawn::<Quitter>(quit_on_start);
    task::yield_now().await;
    task::yield_now().await;

    // Assert
    assert!(!quitter2.is_alive());
}

struct Parent {
    foo: Handle<()>,
    bar: Handle<()>,
}

impl Actor for Parent {
    type Props = ();
    type Msg = Request<(), (Handle<()>, Handle<()>)>;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Parent {
            foo: ctx.spawn::<Foo>(()),
            bar: ctx.spawn::<Bar>(()),
        })
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        msg.reply((self.foo.clone(), self.bar.clone()));

        Ok(())
    }
}

#[allow(clippy::disallowed_names)]
#[tokio::test]
async fn stopping_a_root_actor_stops_all_its_children() {
    // Arrange
    let mut node = Node::default();
    let parent = node.spawn::<Parent>(());

    let (req, res) = req_res(());
    parent.send(req);
    let (foo, bar) = res.recv().await.unwrap();

    assert!(foo.is_alive());
    assert!(bar.is_alive());

    // Act
    parent.stop();
    task::yield_now().await;
    task::yield_now().await;

    // Assert
    assert!(!foo.is_alive());
    assert!(!bar.is_alive());
}

use speare::*;

#[derive(Clone, Debug)]
struct A;

#[derive(Clone, Debug)]
struct B;

#[derive(Clone, Debug)]
struct C;

#[derive(Default, Debug)]
struct Foo {
    a: u64,
    b: u32,
    c: u64,
}

#[async_trait]
impl Process for Foo {
    type Error = ();

    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<A>().await;
        evt.subscribe::<B>().await;
        evt.subscribe::<C>().await;
    }
}

#[async_trait]
impl Handler<A> for Foo {
    type Ok = u64;
    type Err = ();

    async fn handle(&mut self, _msg: A, _ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.a += 1;
        reply(self.a)
    }
}

#[async_trait]
impl Handler<B> for Foo {
    type Ok = u32;
    type Err = ();

    async fn handle(&mut self, _msg: B, _ctx: &Ctx<Self>) -> Reply<u32, ()> {
        self.b += 1;
        reply(self.b)
    }
}

#[async_trait]
impl Handler<C> for Foo {
    type Ok = u64;
    type Err = ();

    async fn handle(&mut self, _msg: C, _ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.c += 1;
        reply(self.c)
    }
}

#[derive(Default, Debug)]
struct Bar {
    a: u64,
    b: u64,
    c: u64,
}

#[async_trait]
impl Handler<A> for Bar {
    type Ok = u64;
    type Err = ();

    async fn handle(&mut self, _msg: A, _ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.a += 1;
        reply(self.a)
    }
}

#[async_trait]
impl Handler<B> for Bar {
    type Ok = u64;
    type Err = ();

    async fn handle(&mut self, _msg: B, _ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.b += 1;
        reply(self.b)
    }
}

#[async_trait]
impl Handler<C> for Bar {
    type Ok = u64;
    type Err = ();

    async fn handle(&mut self, _msg: C, _ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.c += 1;
        reply(self.c)
    }
}

#[async_trait]
impl Process for Bar {
    type Error = ();

    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<A>().await;
        evt.subscribe::<B>().await;
    }
}

#[tokio::test]
async fn publishes_messages_only_to_subscribers() {
    // Arrange
    let node = Node::default();
    let foo_pid = node.spawn(Foo::default()).await;
    let bar_pid = node.spawn(Bar::default()).await;

    // Act
    for _ in 0..3 {
        node.publish(A).await;
    }

    for _ in 0..4 {
        node.publish(B).await;
    }

    for _ in 0..5 {
        node.publish(C).await;
    }

    // Assert
    let foo_a = node.ask(&foo_pid, A).await.unwrap();
    let foo_b = node.ask(&foo_pid, B).await.unwrap();
    let foo_c = node.ask(&foo_pid, C).await.unwrap();

    assert_eq!(foo_a, 4);
    assert_eq!(foo_b, 5);
    assert_eq!(foo_c, 6);

    let bar_a = node.ask(&bar_pid, A).await.unwrap();
    let bar_b = node.ask(&bar_pid, B).await.unwrap();
    let bar_c = node.ask(&bar_pid, C).await.unwrap();

    assert_eq!(bar_a, 4);
    assert_eq!(bar_b, 5);
    assert_eq!(bar_c, 1);
}

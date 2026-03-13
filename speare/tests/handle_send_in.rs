use derive_more::From;
use speare::{Actor, Ctx, Node, Request};
use std::time::Duration;
use tokio::task;

struct Collector {
    items: Vec<String>,
}

#[derive(From)]
enum CollectorMsg {
    Item(String),
    GetItems(Request<(), Vec<String>>),
}

impl Actor for Collector {
    type Props = ();
    type Msg = CollectorMsg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Self { items: vec![] })
    }

    async fn handle(&mut self, msg: CollectorMsg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            CollectorMsg::Item(s) => self.items.push(s),
            CollectorMsg::GetItems(req) => req.reply(self.items.clone()),
        }

        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn send_in_delivers_message_after_delay() {
    // Arrange
    let mut node = Node::default();
    let handle = node.actor::<Collector>(()).spawn();
    task::yield_now().await;

    // Act
    handle.send_in(CollectorMsg::Item("delayed".into()), Duration::from_secs(1));
    task::yield_now().await;

    // not yet delivered
    let items: Vec<String> = handle.req(()).await.unwrap();
    assert!(items.is_empty());

    // advance past the delay
    tokio::time::sleep(Duration::from_secs(1)).await;
    task::yield_now().await;

    // Assert
    let items: Vec<String> = handle.req(()).await.unwrap();
    assert_eq!(items, vec!["delayed"]);
}

#[tokio::test(start_paused = true)]
async fn send_in_does_not_deliver_before_delay() {
    // Arrange
    let mut node = Node::default();
    let handle = node.actor::<Collector>(()).spawn();
    task::yield_now().await;

    // Act
    handle.send_in(CollectorMsg::Item("delayed".into()), Duration::from_secs(5));

    // advance only partway
    tokio::time::sleep(Duration::from_secs(3)).await;
    task::yield_now().await;

    // Assert
    let items: Vec<String> = handle.req(()).await.unwrap();
    assert!(items.is_empty());
}

#[tokio::test(start_paused = true)]
async fn send_in_ordering_with_immediate_send() {
    // Arrange
    let mut node = Node::default();
    let handle = node.actor::<Collector>(()).spawn();
    task::yield_now().await;

    // Act
    handle.send_in(CollectorMsg::Item("second".into()), Duration::from_secs(1));
    handle.send(CollectorMsg::Item("first".into()));

    tokio::time::sleep(Duration::from_secs(2)).await;
    task::yield_now().await;

    // Assert
    let items: Vec<String> = handle.req(()).await.unwrap();
    assert_eq!(items, vec!["first", "second"]);
}

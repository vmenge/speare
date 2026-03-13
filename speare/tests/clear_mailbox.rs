use derive_more::From;
use speare::{Actor, Backoff, Ctx, Limit, Node, Request, Supervision};
use tokio::task;

struct Accumulator {
    items: Vec<String>,
}

#[derive(From)]
enum AccMsg {
    Push(String),
    GetItems(Request<(), Vec<String>>),
}

impl Actor for Accumulator {
    type Props = bool; // clear_on_init
    type Msg = AccMsg;
    type Err = String;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        if *ctx.props() {
            ctx.clear_mailbox();
        }

        Ok(Self { items: vec![] })
    }

    async fn handle(&mut self, msg: AccMsg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            AccMsg::Push(s) => {
                if s == "fail" {
                    return Err("fail".into());
                }
                self.items.push(s);
            }

            AccMsg::GetItems(req) => req.reply(self.items.clone()),
        }

        Ok(())
    }
}

#[tokio::test]
async fn clear_mailbox_discards_pending_messages_on_restart() {
    // Arrange
    let clear_on_init = true;
    let mut node = Node::default();
    let handle = node
        .actor::<Accumulator>(clear_on_init)
        .supervision(Supervision::Restart {
            max: Limit::None,
            backoff: Backoff::None,
        })
        .spawn();
    task::yield_now().await;

    handle.send(AccMsg::Push("a".into()));
    handle.send(AccMsg::Push("b".into()));
    handle.send(AccMsg::Push("fail".into())); // triggers restart
    handle.send(AccMsg::Push("c".into())); // queued before restart completes
    handle.send(AccMsg::Push("d".into()));
    task::yield_now().await;

    // Act & Assert
    // After restart with clear_mailbox, "c" and "d" should be gone
    let items: Vec<String> = handle.req(()).await.unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn without_clear_mailbox_pending_messages_survive_restart() {
    // Arrange
    let clear_on_init = false;
    let mut node = Node::default();
    let handle = node
        .actor::<Accumulator>(clear_on_init)
        .supervision(Supervision::Restart {
            max: Limit::None,
            backoff: Backoff::None,
        })
        .spawn();
    task::yield_now().await;

    handle.send(AccMsg::Push("a".into()));
    handle.send(AccMsg::Push("fail".into())); // triggers restart
    handle.send(AccMsg::Push("b".into())); // survives because no clear_mailbox
    task::yield_now().await;

    // Act & Assert
    // "b" should be processed after restart since mailbox was not cleared
    let items: Vec<String> = handle.req(()).await.unwrap();
    assert_eq!(items, vec!["b"]);
}

#[tokio::test]
async fn new_messages_after_clear_mailbox_are_received() {
    // Arrange
    let clear_on_init = true;
    let mut node = Node::default();
    let handle = node
        .actor::<Accumulator>(clear_on_init)
        .supervision(Supervision::Restart {
            max: Limit::None,
            backoff: Backoff::None,
        })
        .spawn();
    task::yield_now().await;

    // trigger a restart
    handle.send(AccMsg::Push("fail".into()));
    task::yield_now().await;

    // Act - send new messages after restart settles
    handle.send(AccMsg::Push("x".into()));
    handle.send(AccMsg::Push("y".into()));

    // Assert
    let items: Vec<String> = handle.req(()).await.unwrap();
    assert_eq!(items, vec!["x", "y"]);
}

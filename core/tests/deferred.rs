use std::time::Duration;

use speare::*;

struct SayHi;

struct GiveBone;

#[derive(Default)]
struct Dog {
    hi_responder: Option<Responder<Self, SayHi>>,
}

impl Process for Dog {}

#[process]
impl Dog {
    #[handler]
    async fn hi(&mut self, _msg: SayHi, ctx: &Ctx<Self>) -> Reply<String, ()> {
        self.hi_responder = ctx.responder::<SayHi>();

        noreply()
    }

    #[handler]
    async fn get_bone(&mut self, _msg: GiveBone, _ctx: &Ctx<Self>) -> Reply<(), ()> {
        if let Some(responder) = &self.hi_responder {
            responder.reply(Ok("woof".to_string()))
        }

        noreply()
    }
}

#[tokio::test]
async fn dog_responds_when_given_a_bone() {
    // Arrange
    let node = Node::default();
    let dog_pid = node.spawn(Dog::default()).await;

    // Act
    node.tell_in(&dog_pid, GiveBone, Duration::from_millis(10))
        .await;

    let result = node
        .ask(&dog_pid, SayHi)
        .await
        .unwrap_or_else(|_| "".to_string());

    // Assert
    assert_eq!(result, "woof".to_string());
}

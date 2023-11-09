use std::time::Duration;

use speare::*;
use tokio::task;

struct SayHi;

struct GiveBone;

#[derive(Default)]
struct Dog {
    hi_responder: Option<Responder<Self, SayHi>>,
}

impl Process for Dog {}

#[process]
impl Dog {
    // the hi Handler specifies that it returns a String as a response,
    // thus when we call it on get_bone, the Reply itself must be a String.
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

        noreply() // this could also be a reply(()), but noreply() conveys meaning better
    }
}

#[tokio::test]
async fn dog_responds_when_given_a_bone() {
    // Arrange
    let node = Node::default();
    let node_clone = node.clone();

    let dog_pid = node.spawn(Dog::default()).await;
    let dog_pid_clone = dog_pid.clone();

    // Act
    task::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        node_clone.tell(&dog_pid_clone, GiveBone).await;
    });

    let result = node
        .ask(&dog_pid, SayHi)
        .await
        .unwrap_or_else(|_| "".to_string());

    // Assert
    assert_eq!(result, "woof".to_string());
}

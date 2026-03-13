use derive_more::From;
use speare::{Actor, Ctx, Handle, Node, ReqErr, Request};
use std::time::Duration;
use tokio::task;

// -- Actors ------------------------------------------------------------------

struct Echo;

#[derive(From)]
enum EchoMsg {
    Echo(Request<String, String>),
    Add(Request<(u32, u32), u32>),
}

impl Actor for Echo {
    type Props = ();
    type Msg = EchoMsg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Echo)
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            EchoMsg::Echo(req) => req.reply(req.data().clone()),
            EchoMsg::Add(req) => {
                let (a, b) = req.data();
                req.reply(a + b);
            }
        }

        Ok(())
    }
}

/// Actor that never replies to requests (to test Dropped error).
struct BlackHole;

impl Actor for BlackHole {
    type Props = ();
    type Msg = Request<(), ()>;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(BlackHole)
    }

    async fn handle(&mut self, _msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        // intentionally drop request without replying
        Ok(())
    }
}

/// Actor that delays before replying (to test timeout).
struct SlowReplier;

#[derive(From)]
enum SlowMsg {
    Slow(Request<Duration, String>),
}

impl Actor for SlowReplier {
    type Props = ();
    type Msg = SlowMsg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(SlowReplier)
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            SlowMsg::Slow(req) => {
                let delay = *req.data();
                tokio::time::sleep(delay).await;
                req.reply("done".into());
            }
        }

        Ok(())
    }
}

/// Actor whose message type doesn't implement From<Request> — forces use of reqw.
struct ManualWrap {
    count: u32,
}

enum ManualMsg {
    Inc,
    GetCount(Request<(), u32>),
}

impl Actor for ManualWrap {
    type Props = ();
    type Msg = ManualMsg;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(ManualWrap { count: 0 })
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ManualMsg::Inc => self.count += 1,
            ManualMsg::GetCount(req) => req.reply(self.count),
        }

        Ok(())
    }
}

/// Actor that forwards requests to another actor.
struct Forwarder {
    target: Handle<EchoMsg>,
}

#[derive(From)]
enum ForwarderMsg {
    Forward(Request<String, String>),
}

impl Actor for Forwarder {
    type Props = Handle<EchoMsg>;
    type Msg = ForwarderMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Forwarder {
            target: ctx.props().clone(),
        })
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ForwarderMsg::Forward(req) => {
                let result: String = self.target.req(req.data().clone()).await.unwrap();
                req.reply(result);
            }
        }

        Ok(())
    }
}

// -- Tests -------------------------------------------------------------------

#[tokio::test]
async fn req_returns_response_from_actor() {
    // Arrange
    let mut node = Node::default();
    let echo = node.actor::<Echo>(()).spawn();
    task::yield_now().await;

    // Act
    let result: String = echo.req("hello".to_string()).await.unwrap();

    // Assert
    assert_eq!(result, "hello");
}

#[tokio::test]
async fn req_works_with_complex_data() {
    // Arrange
    let mut node = Node::default();
    let echo = node.actor::<Echo>(()).spawn();
    task::yield_now().await;

    // Act
    let sum: u32 = echo.req((3u32, 7u32)).await.unwrap();

    // Assert
    assert_eq!(sum, 10);
}

#[tokio::test]
async fn req_returns_dropped_when_actor_never_replies() {
    // Arrange
    let mut node = Node::default();
    let hole = node.actor::<BlackHole>(()).spawn();
    task::yield_now().await;

    // Act
    let result = hole.req(()).await;

    // Assert
    assert!(matches!(result, Err(ReqErr::Dropped)));
}

#[tokio::test]
async fn req_returns_dropped_when_actor_is_stopped() {
    // Arrange
    let mut node = Node::default();
    let echo = node.actor::<Echo>(()).spawn();
    task::yield_now().await;

    let (req, res) = speare::req_res::<String, String>("hello".into());
    echo.stop();
    task::yield_now().await;

    // Send after stop — request is never handled
    echo.send(req);

    // Act
    let result = res.recv().await;

    // Assert
    assert!(matches!(result, Err(ReqErr::Dropped)));
}

#[tokio::test]
async fn req_timeout_returns_response_within_deadline() {
    // Arrange
    let mut node = Node::default();
    let slow = node.actor::<SlowReplier>(()).spawn();
    task::yield_now().await;

    // Act
    let result: String = slow
        .req_timeout(Duration::from_millis(5), Duration::from_secs(1))
        .await
        .unwrap();

    // Assert
    assert_eq!(result, "done");
}

#[tokio::test]
async fn req_timeout_returns_timeout_when_deadline_exceeded() {
    // Arrange
    let mut node = Node::default();
    let slow = node.actor::<SlowReplier>(()).spawn();
    task::yield_now().await;

    // Act
    let result = slow
        .req_timeout(Duration::from_secs(10), Duration::from_millis(10))
        .await;

    // Assert
    assert!(matches!(result, Err(ReqErr::Timeout)));
}

#[tokio::test]
async fn reqw_sends_request_using_wrapper_function() {
    // Arrange
    let mut node = Node::default();
    let actor = node.actor::<ManualWrap>(()).spawn();
    task::yield_now().await;

    actor.send(ManualMsg::Inc);
    actor.send(ManualMsg::Inc);
    actor.send(ManualMsg::Inc);
    task::yield_now().await;

    // Act
    let count: u32 = actor.reqw(ManualMsg::GetCount, ()).await.unwrap();

    // Assert
    assert_eq!(count, 3);
}

#[tokio::test]
async fn reqw_timeout_returns_response_within_deadline() {
    // Arrange
    let mut node = Node::default();
    let actor = node.actor::<ManualWrap>(()).spawn();
    task::yield_now().await;

    actor.send(ManualMsg::Inc);
    task::yield_now().await;

    // Act
    let count: u32 = actor
        .reqw_timeout(ManualMsg::GetCount, (), Duration::from_secs(1))
        .await
        .unwrap();

    // Assert
    assert_eq!(count, 1);
}

#[tokio::test]
async fn multiple_sequential_requests_to_same_actor() {
    // Arrange
    let mut node = Node::default();
    let echo = node.actor::<Echo>(()).spawn();
    task::yield_now().await;

    // Act & Assert
    for i in 0..10 {
        let msg = format!("msg-{i}");
        let result: String = echo.req(msg.clone()).await.unwrap();
        assert_eq!(result, msg);
    }
}

#[tokio::test]
async fn concurrent_requests_from_multiple_callers() {
    // Arrange
    let mut node = Node::default();
    let echo = node.actor::<Echo>(()).spawn();
    task::yield_now().await;

    // Act
    let mut handles = Vec::new();
    for i in 0..10 {
        let echo = echo.clone();
        handles.push(tokio::spawn(async move {
            let msg = format!("caller-{i}");
            let result: String = echo.req(msg.clone()).await.unwrap();
            (result, msg)
        }));
    }

    // Assert
    for h in handles {
        let (result, expected) = h.await.unwrap();
        assert_eq!(result, expected);
    }
}

#[tokio::test]
async fn actor_can_make_request_to_another_actor_inside_handle() {
    // Arrange
    let mut node = Node::default();
    let echo = node.actor::<Echo>(()).spawn();
    let forwarder = node.actor::<Forwarder>(echo).spawn();
    task::yield_now().await;

    // Act
    let result: String = forwarder.req("forwarded".to_string()).await.unwrap();

    // Assert
    assert_eq!(result, "forwarded");
}

#[tokio::test]
async fn response_recv_timeout_returns_dropped_when_request_dropped() {
    // Arrange
    let (req, res) = speare::req_res::<(), String>(());
    drop(req);

    // Act
    let result = res.recv_timeout(Duration::from_secs(1)).await;

    // Assert
    assert!(matches!(result, Err(ReqErr::Dropped)));
}

#[tokio::test]
async fn request_data_is_accessible() {
    // Arrange
    let (req, _res) = speare::req_res::<String, ()>("payload".to_string());

    // Act & Assert
    assert_eq!(req.data(), "payload");
}

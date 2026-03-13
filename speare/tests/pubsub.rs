mod sync_vec;

use derive_more::From;
use speare::{Actor, Backoff, Ctx, Limit, Node, PubSubError, Supervision};
use sync_vec::SyncVec;
use tokio::task;

// -- shared event type --------------------------------------------------------

#[derive(Clone)]
struct OrderPlaced {
    id: u32,
}

// -- Subscriber actor ---------------------------------------------------------

struct OrderSubscriber;

#[derive(From)]
enum OrderSubscriberMsg {
    Order(OrderPlaced),
}

impl Actor for OrderSubscriber {
    type Props = SyncVec<u32>;
    type Msg = OrderSubscriberMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.subscribe::<OrderPlaced>("orders").unwrap();
        Ok(OrderSubscriber)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            OrderSubscriberMsg::Order(o) => ctx.props().push(o.id).await,
        }

        Ok(())
    }
}

// -- Publisher actor ----------------------------------------------------------

struct OrderPublisher;

impl Actor for OrderPublisher {
    type Props = ();
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.publish("orders", OrderPlaced { id: 42 }).unwrap();
        Ok(OrderPublisher)
    }
}

// -- Tests --------------------------------------------------------------------

#[tokio::test]
async fn publish_delivers_to_subscriber() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<u32>::default();
    node.actor::<OrderSubscriber>(recvd.clone()).spawn();
    task::yield_now().await;

    // Act
    node.publish("orders", OrderPlaced { id: 42 }).unwrap();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![42]);
}

#[tokio::test]
async fn publish_delivers_to_multiple_subscribers() {
    // Arrange
    let mut node = Node::default();
    let recvd_a = SyncVec::<u32>::default();
    let recvd_b = SyncVec::<u32>::default();
    node.actor::<OrderSubscriber>(recvd_a.clone()).spawn();
    node.actor::<OrderSubscriber>(recvd_b.clone()).spawn();
    task::yield_now().await;

    // Act
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd_a.clone_vec().await, vec![42]);
    assert_eq!(recvd_b.clone_vec().await, vec![42]);
}

#[tokio::test]
async fn publish_to_empty_topic_is_noop() {
    // Arrange
    let mut node = Node::default();

    struct Pub;
    impl Actor for Pub {
        type Props = SyncVec<bool>;
        type Msg = ();
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            let result = ctx.publish("nobody-listens", OrderPlaced { id: 1 });
            ctx.props().push(result.is_ok()).await;

            Ok(Pub)
        }
    }

    let recvd = SyncVec::<bool>::default();

    // Act
    node.actor::<Pub>(recvd.clone()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![true]);
}

#[tokio::test]
async fn subscribe_type_mismatch_returns_error() {
    // Arrange
    let mut node = Node::default();
    node.actor::<OrderSubscriber>(SyncVec::default()).spawn();
    task::yield_now().await;

    struct WrongTypeSub;
    impl Actor for WrongTypeSub {
        type Props = SyncVec<bool>;
        type Msg = String;
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            let result = ctx.subscribe::<String>("orders");
            ctx.props()
                .push(matches!(result, Err(PubSubError::TypeMismatch { .. })))
                .await;

            Ok(WrongTypeSub)
        }
    }

    let recvd = SyncVec::<bool>::default();

    // Act
    node.actor::<WrongTypeSub>(recvd.clone()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![true]);
}

#[tokio::test]
async fn publish_type_mismatch_returns_error() {
    // Arrange
    let mut node = Node::default();
    node.actor::<OrderSubscriber>(SyncVec::default()).spawn();
    task::yield_now().await;

    struct WrongTypePub;
    impl Actor for WrongTypePub {
        type Props = SyncVec<bool>;
        type Msg = ();
        type Err = ();

        async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
            let result = ctx.publish("orders", "not an OrderPlaced".to_string());
            ctx.props()
                .push(matches!(result, Err(PubSubError::TypeMismatch { .. })))
                .await;

            Ok(WrongTypePub)
        }
    }

    let recvd = SyncVec::<bool>::default();

    // Act
    node.actor::<WrongTypePub>(recvd.clone()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![true]);
}

// -- Cleanup on stop ----------------------------------------------------------

#[tokio::test]
async fn subscriptions_cleaned_up_on_stop() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<u32>::default();
    let sub_handle = node.actor::<OrderSubscriber>(recvd.clone()).spawn();
    task::yield_now().await;

    // Act
    sub_handle.stop();
    task::yield_now().await;
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, Vec::<u32>::new());
}

// -- Cleanup on restart -------------------------------------------------------

struct RestartSubscriber;

#[derive(Clone)]
struct TriggerFail;

#[derive(From)]
enum RestartSubMsg {
    Order(OrderPlaced),
    Fail(TriggerFail),
}

impl Actor for RestartSubscriber {
    type Props = SyncVec<u32>;
    type Msg = RestartSubMsg;
    type Err = String;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.subscribe::<OrderPlaced>("orders").unwrap();
        Ok(RestartSubscriber)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            RestartSubMsg::Order(o) => {
                ctx.props().push(o.id).await;
                Ok(())
            }

            RestartSubMsg::Fail(_) => Err("forced".to_string()),
        }
    }
}

#[tokio::test]
async fn subscriptions_cleaned_up_on_restart_no_duplicates() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<u32>::default();
    let sub_handle = node
        .actor::<RestartSubscriber>(recvd.clone())
        .supervision(Supervision::Restart {
            max: Limit::None,
            backoff: Backoff::None,
        })
        .spawn();
    task::yield_now().await;

    // Act
    sub_handle.send(TriggerFail);
    task::yield_now().await;
    task::yield_now().await; // extra yield for restart cycle
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![42]);
}

// -- Dynamic subscribe from handle() -----------------------------------------

struct LazySubscriber;

struct DoSubscribe;

#[derive(From)]
enum LazySubMsg {
    Subscribe(DoSubscribe),
    Order(OrderPlaced),
}

impl Actor for LazySubscriber {
    type Props = SyncVec<u32>;
    type Msg = LazySubMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(LazySubscriber)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            LazySubMsg::Subscribe(_) => {
                ctx.subscribe::<OrderPlaced>("orders").unwrap();
            }

            LazySubMsg::Order(o) => {
                ctx.props().push(o.id).await;
            }
        }

        Ok(())
    }
}

#[tokio::test]
async fn subscribe_from_handle_works() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<u32>::default();
    let sub_handle = node.actor::<LazySubscriber>(recvd.clone()).spawn();
    task::yield_now().await;

    // Act & Assert

    // Not subscribed yet — publish should not deliver
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;
    assert_eq!(recvd.clone_vec().await, Vec::<u32>::new());

    // Now subscribe dynamically
    sub_handle.send(DoSubscribe);
    task::yield_now().await;

    // Publish again — should deliver this time
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;
    assert_eq!(recvd.clone_vec().await, vec![42]);
}

// -- Double subscribe to same topic -------------------------------------------

struct DoubleSubscriber;

#[derive(From)]
enum DoubleSubMsg {
    Order(OrderPlaced),
}

impl Actor for DoubleSubscriber {
    type Props = SyncVec<u32>;
    type Msg = DoubleSubMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.subscribe::<OrderPlaced>("orders").unwrap();
        ctx.subscribe::<OrderPlaced>("orders").unwrap();
        Ok(DoubleSubscriber)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            DoubleSubMsg::Order(o) => ctx.props().push(o.id).await,
        }
        Ok(())
    }
}

#[tokio::test]
async fn double_subscribe_delivers_message_twice() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<u32>::default();
    node.actor::<DoubleSubscriber>(recvd.clone()).spawn();
    task::yield_now().await;

    // Act
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![42, 42]);
}

#[tokio::test]
async fn double_subscribe_cleaned_up_fully_on_stop() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<u32>::default();
    let handle = node.actor::<DoubleSubscriber>(recvd.clone()).spawn();
    task::yield_now().await;

    // Act
    handle.stop();
    task::yield_now().await;
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd.clone_vec().await, Vec::<u32>::new());
}

// -- Stale subscriber (channel closed) ----------------------------------------

#[tokio::test]
async fn publish_ignores_dead_subscriber_channels() {
    // Arrange
    let mut node = Node::default();
    let recvd_alive = SyncVec::<u32>::default();
    let recvd_dead = SyncVec::<u32>::default();
    let dead_handle = node.actor::<OrderSubscriber>(recvd_dead.clone()).spawn();
    node.actor::<OrderSubscriber>(recvd_alive.clone()).spawn();
    task::yield_now().await;

    // Act
    dead_handle.stop();
    task::yield_now().await;
    node.actor::<OrderPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(recvd_alive.clone_vec().await, vec![42]);
    assert_eq!(recvd_dead.clone_vec().await, Vec::<u32>::new());
}

// -- Multi-topic per actor ----------------------------------------------------

#[derive(Clone)]
struct PaymentProcessed {
    amount: u32,
}

struct MultiTopicSubscriber;

#[derive(From)]
enum MultiTopicMsg {
    Order(OrderPlaced),
    Payment(PaymentProcessed),
}

impl Actor for MultiTopicSubscriber {
    type Props = (SyncVec<u32>, SyncVec<u32>);
    type Msg = MultiTopicMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.subscribe::<OrderPlaced>("orders").unwrap();
        ctx.subscribe::<PaymentProcessed>("payments").unwrap();
        Ok(MultiTopicSubscriber)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            MultiTopicMsg::Order(o) => ctx.props().0.push(o.id).await,
            MultiTopicMsg::Payment(p) => ctx.props().1.push(p.amount).await,
        }
        Ok(())
    }
}

struct PaymentPublisher;

impl Actor for PaymentPublisher {
    type Props = ();
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.publish("payments", PaymentProcessed { amount: 100 })
            .unwrap();
        Ok(PaymentPublisher)
    }
}

#[tokio::test]
async fn actor_subscribes_to_multiple_topics() {
    // Arrange
    let mut node = Node::default();
    let orders = SyncVec::<u32>::default();
    let payments = SyncVec::<u32>::default();
    node.actor::<MultiTopicSubscriber>((orders.clone(), payments.clone()))
        .spawn();
    task::yield_now().await;

    // Act
    node.actor::<OrderPublisher>(()).spawn();
    node.actor::<PaymentPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(orders.clone_vec().await, vec![42]);
    assert_eq!(payments.clone_vec().await, vec![100]);
}

#[tokio::test]
async fn multi_topic_subscriptions_cleaned_up_on_stop() {
    // Arrange
    let mut node = Node::default();
    let orders = SyncVec::<u32>::default();
    let payments = SyncVec::<u32>::default();
    let handle = node
        .actor::<MultiTopicSubscriber>((orders.clone(), payments.clone()))
        .spawn();
    task::yield_now().await;

    // Act
    handle.stop();
    task::yield_now().await;
    node.actor::<OrderPublisher>(()).spawn();
    node.actor::<PaymentPublisher>(()).spawn();
    task::yield_now().await;

    // Assert
    assert_eq!(orders.clone_vec().await, Vec::<u32>::new());
    assert_eq!(payments.clone_vec().await, Vec::<u32>::new());
}

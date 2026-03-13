# Pub/Sub

`speare` includes a built-in publish/subscribe system shared across the entire `Node` tree. It lets actors communicate through named topics without needing direct `Handle` references. Publishers and subscribers are fully decoupled -- a publisher does not know who is listening, and a subscriber does not know who is publishing.

## Subscribing to a Topic

Call `ctx.subscribe::<T>(topic)` inside `init` or `handle` to receive messages published to that topic. The type parameter `T` is the message type for the topic. Published messages are cloned, converted via `From<T>` into the actor's `Msg` type, and delivered to its mailbox.

```rust,ignore
use speare::*;
use derive_more::From;

#[derive(Clone)]
struct OrderPlaced {
    id: u32,
}

struct OrderSubscriber;

#[derive(From)]
enum OrderSubMsg {
    Order(OrderPlaced),
}

impl Actor for OrderSubscriber {
    type Props = ();
    type Msg = OrderSubMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.subscribe::<OrderPlaced>("orders").unwrap();
        Ok(OrderSubscriber)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            OrderSubMsg::Order(o) => println!("Received order {}", o.id),
        }

        Ok(())
    }
}
```

The `From` derive is required -- it is how speare converts the topic's message type into the actor's `Msg` enum. This is the same `From` trick described in the [Communication](communication.md) chapter.

## Publishing to a Topic

Call `ctx.publish(topic, msg)` to send a message to all subscribers of a topic. The message is cloned for each subscriber.

```rust,ignore
ctx.publish("orders", OrderPlaced { id: 42 }).unwrap();
```

Publishing to a topic with no subscribers is a no-op and returns `Ok(())`.

You can also publish from a `Node` directly:

```rust,ignore
node.publish("orders", OrderPlaced { id: 42 }).unwrap();
```

## Type Safety

Each topic is locked to a single message type. The first `subscribe` or `publish` call on a topic sets its type. Any subsequent call with a different type returns `Err(PubSubError::TypeMismatch { .. })`.

```rust,ignore
// First subscribe locks "orders" to OrderPlaced
ctx.subscribe::<OrderPlaced>("orders").unwrap();

// This fails -- "orders" is already locked to OrderPlaced, not String
let result = ctx.subscribe::<String>("orders");
assert!(matches!(result, Err(PubSubError::TypeMismatch { .. })));
```

The same applies to `publish`:

```rust,ignore
// Topic "orders" is locked to OrderPlaced by a subscriber
let result = ctx.publish("orders", "not an OrderPlaced".to_string());
assert!(matches!(result, Err(PubSubError::TypeMismatch { .. })));
```

## Multiple Topics

An actor can subscribe to multiple topics. Each subscription needs a corresponding variant in the actor's `Msg` enum with a `From` implementation.

```rust,ignore
#[derive(Clone)]
struct PaymentProcessed {
    amount: u32,
}

#[derive(From)]
enum MultiMsg {
    Order(OrderPlaced),
    Payment(PaymentProcessed),
}

impl Actor for MultiSubscriber {
    type Props = ();
    type Msg = MultiMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.subscribe::<OrderPlaced>("orders").unwrap();
        ctx.subscribe::<PaymentProcessed>("payments").unwrap();
        Ok(MultiSubscriber)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            MultiMsg::Order(o) => println!("Order {}", o.id),
            MultiMsg::Payment(p) => println!("Payment ${}", p.amount),
        }

        Ok(())
    }
}
```

## Dynamic Subscription

Subscribing is not limited to `init`. You can subscribe from `handle` as well, letting an actor join a topic in response to a message:

```rust,ignore
async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
    match msg {
        Msg::StartListening => {
            ctx.subscribe::<OrderPlaced>("orders").unwrap();
        }

        Msg::Order(o) => {
            println!("Got order {}", o.id);
        }
    }

    Ok(())
}
```

## Automatic Cleanup

Subscriptions are automatically cleaned up when an actor stops -- whether by error, parent shutdown, or an explicit `handle.stop()` call. Once stopped, a subscriber will no longer receive messages from any topic it was subscribed to.

Subscriptions are also cleaned up before a restart. When an actor with `Supervision::Restart` fails and restarts, its old subscriptions are removed before `init` runs again. This means `init` can re-subscribe without causing duplicate deliveries.

## Error Handling

All pub/sub operations return `Result<(), PubSubError>`. The enum has two variants:

```rust,ignore
pub enum PubSubError {
    /// Topic exists with a different message type.
    TypeMismatch { topic: String },
    /// Internal lock poisoned (rare -- indicates a panic elsewhere).
    PoisonErr,
}
```

`PubSubError` implements `Display` and `Error`, so it works with `?` and standard error handling.

In practice, `TypeMismatch` is the variant you will encounter. `PoisonErr` only occurs if another thread panicked while holding the pub/sub lock, which should not happen under normal operation.

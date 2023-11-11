# Pub / Sub

In `speare`, the Publish / Subscribe pattern is implemented with a focus on explicit subscriptions. While every `Process` with a `Handler` for message `M` can handle direct messages, subscribing to global publishes of `M` is a conscious choice. This design allows handlers to be public, reachable by anyone with access to the message type, as opposed to being limited to interactions via a specific `Pid<P>`.

## Subscription Mechanism

To subscribe to global publishes, the message must implement `Clone`. Here’s how it works:

### **Defining a Subscribable Message** 
Create a message type that implements `Clone`.

```rust
#[derive(Clone)]
struct SayHi;
```

### **Implementing Subscriptions**
In the `Process` implementation, use the `#[subscriptions]` attribute to define subscriptions. It is only possible to subscribe to messages which have a `Handler` implemented.

```rust
struct Cat;

#[process]
impl Cat {
    #[subscriptions]
    async fn subs(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }

    #[handler]
    async fn hi(&mut self, msg: SayHi) -> Reply<(), ()> {
        println!("MEOW!");
        reply(())
    }
}

struct Dog;

#[process]
impl Dog {
    #[subscriptions]
    async fn subs(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }

    #[handler]
    async fn hi(&mut self, msg: SayHi) -> Reply<(), ()> {
        println!("WOOF!");
        reply(())
    }
}
```

### **Publishing Messages**
Messages can be globally published, reaching all subscribed processes.

```rust
#[tokio::main]
async fn main() {
    let node = Node::default();
    node.spawn(Cat).await;
    node.spawn(Dog).await;

    node.publish(SayHi).await;  
    // Both Cat and Dog will respond, not necessarily in the same order every time
    // "WOOF!"
    // "MEOW!"
}
```

In this example, both `Cat` and `Dog` processes subscribe to `SayHi` messages. When `SayHi` is published, each subscribed process's handler is invoked, allowing for a decoupled yet coordinated response across different processes. This Pub / Sub mechanism enhances the flexibility of message passing in `speare`, enabling broad communication without direct process references.

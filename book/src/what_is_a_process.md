# What is a Process?
A Process is really just a loop that lives in a `tokio::task` together with some state, yielding to the tokio runtime while it waits for a message received via a flume channel which it then uses to modify its state. All `speare` does is provide mechanisms to manage lifecycle, communication and supervision of processes.

A `Process` is comprised of a few parts:
### Trait Implementor
- **State**: the main struct which implements the `Process` trait. The state is mutable and can be modified every time a messaged is received by the `Process`.

### Associated types
- **Props**: dependencies needed by the struct that implements `Process` for instantiation, configuration or anything in between.
- **Msg**: data received and processed by the `Process`.
- **Err**: the error that can be returned by the process when it fails.

### Trait Functions
- **async fn init**: `(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err>`

    *Required*. Used by `speare` to create new instances of your `Process` whenever it is spawned or restarted.

- **async fn exit**: `(&mut self, reason: ExitReason<Self>, ctx: &mut Ctx<Self>)`

    *Optional*. Called every time your `Process` is stopped, be it manually through its `Handle<_>` or
    by the parent through its [supervision strategy](./supervision.md).

- **handle**: `(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err>`

    *Optional*. Called when the `Process` receives a message through its channel. 
    Messages are **always** processed sequentially.

- **supervision**: `(props: &Self::Props) -> Supervision`

    *Optional*. Called before your `Process` is spawned. Used to customize the [supervision strategy](./supervision.md)
    for its **children**. If not implemented, the default supervision strategy will
    be used: one-for-one infinite restarts without backoff.


## Example
Below is an example of a process making use of all its associated types and trait functions.

```rs
use speare::*;
use async_trait::async_trait;

struct Counter {
    count: u32,
}

struct CounterProps {
    initial_count: u32,
    max_count: u32,
    min_count: u32,
}

enum CounterMsg {
    Add(u32),
    Subtract(u32),
}

#[derive(Debug)]
enum CounterErr {
    MaxCountExceeded,
    MinCountExceeded,
}

#[async_trait]
impl Process for Counter {
    type Props = CounterProps;
    type Msg = CounterMsg;
    type Err = CounterErr;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        println!("Counter starting up!");

        Ok(Counter {
            count: ctx.props().initial_count,
        })
    }

    async fn exit(&mut self, reason: ExitReason<Self>, ctx: &mut Ctx<Self>) {
        println!("Counter exiting due to: {:?}", reason);
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            CounterMsg::Add(n) => {
                self.count += n;
                if self.count > ctx.props().max_count {
                    return Err(CounterErr::MaxCountExceeded);
                }
            }

            CounterMsg::Subtract(n) => {
                self.count -= n;
                if self.count < ctx.props().max_count {
                    return Err(CounterErr::MinCountExceeded);
                }
            }
        }

        Ok(())
    }

    fn supervision(props: &Self::Props) -> Supervision {
        Supervision::one_for_one().directive(Directive::Restart)
    }
}

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    let counter = node.spawn::<Counter>(CounterProps {
        initial_count: 5,
        max_count: 20,
        min_count: 5
    });

    counter.send(CounterMsg::Add(5));
    counter.send(CounterMsg::Subtract(2));
}
```

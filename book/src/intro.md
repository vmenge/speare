# Introduction
This guide introduces `speare`, covering `Process` creation, message handling, and essential library features. The objective is to provide clear, concise instructions for effectively utilizing `speare` in your Rust projects.

## What is `speare`?
`speare` is a Rust library designed to simplify the process of actor-based concurrency. It provides an abstraction over [tokio green threads](https://tokio.rs/tokio/tutorial/spawning#tasks) and [flume channels](https://github.com/zesterer/flume), allowing for easier management of tokio threads and efficient message passing between them. `speare` revolves around a main abstraction: `Process` -- which lives on its own green thread owning its data, reducing the risks of deadlocks and encouraging a modular design. No more `Arc<Mutex<T>>` everywhere :)

## Quick Look
Below is an example of a very minimal `Counter` `Process`.

```rust
use speare::*;
use async_trait::async_trait;
use tokio::time;

struct Counter {
    count: u32,
}

enum CounterMsg {
    Add(u32),
    Subtract(u32),
    Print
}

#[async_trait]
impl Process for Counter {
    type Props = ();
    type Msg = CounterMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Counter { count: 0 })
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            CounterMsg::Add(n) => self.count += n,
            CounterMsg::Subtract(n) => self.count -= n,
            CounterMsg::Print => println!("Count is {}", self.count)
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    let counter = node.spawn::<Counter>(());
    counter.send(CounterMsg::Add(5));
    counter.send(CounterMsg::Subtract(2));
    counter.send(CounterMsg::Print); // will print 3

    // We wait so the program doesn't end before we print.
    time::sleep(Duration::from_millis(1)).await;
}
```

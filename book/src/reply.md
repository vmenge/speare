# Reply<T,E>

`Reply<T, E>` is the mandatory return type for methods annotated with the `#[handler]` proc macro. It's an alias for `Result<Option<T>, E>`, where `T` is the type of a successful return value, and `E` is the error type.
The two main reasons `Reply` was implemented as an alias were:
- We can represent abscence of a reply by using `Ok(None)`.
- We can leverage the `Try` trait and use the `?` operator.

## Helper Functions: `reply()` and `noreply()`
`speare` provides two helper functions that should be used when returning values from handlers:
- `reply()`: This function is used to easily create a successful `Reply`. It wraps the return value in `Some`, then in `Ok`.
- `noreply()`: Use this when you don't need to send back any data. It essentially returns `Ok(None)`.

> **Any `Handler` that returns `noreply()` will return an `AskErr::NoReply` to `.ask` calls, unless [deferred replies](./reply.md#deferring-replies) are implemented.** This means that more often than not you might find it better to 
return with `reply(())` than `noreply()`, unless you want to explicitly forbid `.ask` calls.

## Early Exit with `?`
Since `Reply<T, E>` is a `Result`, you can use the `?` operator for early exit in error scenarios. This can simplify error handling in your handlers.
```rust
#[handler]
async fn process_message(&mut self, msg: MyMessage) -> Reply<MyType, MyError> {
    let result = some_operation().await?;

    reply(result)
}
```

## Deferring Replies
In `speare`, you can delay responses to messages. This is done by using `noreply()` in your `#[handler]` and storing a `Responder` for later use. 

> It's important to remember that if you don't store the `Responder` and use `noreply()`, any `.ask()` calls to that `#[handler]` will fail immediately with `AskErr::NoReply`, as it expects a reply which isn't provided. 

This technique allows for more flexible response patterns, like in the example below where `Dog` responds to a `SayHi` message only after receiving a `GiveBone` message.

```rust
use speare::*;

struct SayHi;
struct GiveBone;

#[derive(Default)]
struct Dog {
    hi_responder: Option<Responder<Self, SayHi>>,
}

#[process]
impl Dog {
    // the hi Handler specifies that it returns a String as a response,
    // thus when we use the Responder on get_bone, the Reply sent through 
    // it must be a String.
    #[handler]
    async fn hi(&mut self, _msg: SayHi, ctx: &Ctx<Self>) -> Reply<String, ()> {
        self.hi_responder = ctx.responder::<SayHi>();

        noreply()
    }

    #[handler]
    async fn get_bone(&mut self, _msg: GiveBone) -> Reply<(), ()> {
        if let Some(responder) = &self.hi_responder {
            responder.reply(Ok("Hello".to_string()))
        }

        reply(())
    }
}
```
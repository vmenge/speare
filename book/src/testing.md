# Testing
Testing in `speare` is pretty straightforward thanks to Rust's flexible type system and trait bounds. 

## Implementing Stub / Mock Processes

By defining mock processes that mimic the behavior of actual processes, you can simulate real-world interactions in a test environment.

### Example

Consider a `Process` `MyProcess` that depends on another `Process`. We can create a mock version of the dependent `Process` for testing:

```rust
use speare::*;

struct GetReceivedMsgs;
struct DoSomething;

#[derive(Default)]
struct MockProc {
    received: Vec<String>,
}

#[process]
impl MockProc {
    #[handler]
    async fn handle_msg(&mut self, msg: String) -> Reply<(), ()> {
        self.received.push(msg);

        reply(())
    }

    #[handler]
    async fn get_msgs(&mut self, _: GetReceivedMsgs) -> Reply<Vec<String>, ()> {
        reply(self.received.clone())
    }
}

struct MyProcess<P> {
    other_pid: Pid<P>,
}

#[process]
impl<P> MyProcess<P>
where
    P: Handler<String>,
{
    pub fn new(other_pid: Pid<P>) -> Self {
        Self { other_pid }
    }

    #[handler]
    async fn do_something(&mut self, _: DoSomething, ctx: &Ctx<Self>) -> Reply<(), ()> {
        ctx.tell(&self.other_pid, "something".to_string()).await;

        reply(())
    }
}

#[tokio::test]
async fn test_my_service_with_mock_dependency() {
    // Arrange
    let node = Node::default();
    let mock_pid = node.spawn(MockProc::default()).await;
    let proc_pid = node.spawn(MyProcess::new(mock_pid.clone())).await;

    // Act
    node.tell(&proc_pid, DoSomething).await;

    // Assert
    let actual = node
        .ask(&mock_pid, GetReceivedMsgs)
        .await
        .unwrap_or_default();

    let expected = vec!["something".to_string()];
    assert_eq!(actual, expected);
}
```

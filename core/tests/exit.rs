use speare::*;
use std::time::Duration;

struct MyProc;

#[process(Error = String)]
impl MyProc {}

struct GetErr;

struct Supervisor {
    child: Pid<MyProc>,
    err: Option<String>,
}

#[process]
impl Supervisor {
    pub fn new(child: &Pid<MyProc>) -> Self {
        Self {
            child: child.clone(),
            err: None,
        }
    }

    #[on_init]
    async fn on_init(&mut self, ctx: &Ctx<Self>) {
        ctx.monitor(&self.child);
    }

    #[handler]
    async fn handle_my_proc_exit(
        &mut self,
        signal: ExitSignal<MyProc>,
        _: &Ctx<Self>,
    ) -> Reply<(), ()> {
        if let ExitReason::Err(err) = signal.reason() {
            self.err = Some(err.to_string());
        }

        noreply()
    }

    #[handler]
    async fn get_err(&mut self, _: GetErr, _: &Ctx<Self>) -> Reply<Option<String>, ()> {
        reply(self.err.take())
    }
}

struct Supervisor2 {
    child: Pid<MyProc>,
    err: Option<String>,
}

#[process]
impl Supervisor2 {
    pub fn new(child: &Pid<MyProc>) -> Self {
        Self {
            child: child.clone(),
            err: None,
        }
    }

    #[on_init]
    async fn on_init(&mut self, ctx: &Ctx<Self>) {
        ctx.monitor(&self.child);
    }

    #[handler]
    async fn handle_my_proc_exit(
        &mut self,
        signal: ExitSignal<MyProc>,
        _: &Ctx<Self>,
    ) -> Reply<(), ()> {
        if let ExitReason::Err(err) = signal.reason() {
            self.err = Some(err.to_string());
        }

        noreply()
    }

    #[handler]
    async fn get_err(&mut self, _: GetErr, _: &Ctx<Self>) -> Reply<Option<String>, ()> {
        reply(self.err.take())
    }
}

#[tokio::test]
async fn exit_kills_process_gracefully() {
    // Arrange
    let node = Node::default();
    let pid = node.spawn(MyProc).await;

    // Act
    let is_alive_before_exit = node.is_alive(&pid);
    node.exit(&pid, ExitReason::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(0)).await;
    let is_alive_after_exit = node.is_alive(&pid);

    // Assert
    assert!(is_alive_before_exit);
    assert!(!is_alive_after_exit);
}

#[tokio::test]
async fn supervisors_are_notified_of_child_exit() {
    // Arrange
    let node = Node::default();
    let child_pid = node.spawn(MyProc).await;
    let supervisor_pid = node.spawn(Supervisor::new(&child_pid)).await;
    let supervisor2_pid = node.spawn(Supervisor2::new(&child_pid)).await;
    tokio::time::sleep(Duration::from_millis(0)).await;

    // Act
    node.exit(&child_pid, ExitReason::Err("bla".to_string()))
        .await;

    tokio::time::sleep(Duration::from_millis(0)).await;

    let actual = node
        .ask(&supervisor_pid, GetErr)
        .await
        .unwrap_or(None)
        .unwrap();

    let actual2 = node
        .ask(&supervisor2_pid, GetErr)
        .await
        .unwrap_or(None)
        .unwrap();

    // Assert
    assert_eq!(actual, "bla".to_string());
    assert_eq!(actual2, "bla".to_string());
}

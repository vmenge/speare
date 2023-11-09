use std::time::Duration;

use speare::*;

struct MyProc;

impl Process for MyProc {}

#[tokio::test]
async fn exit_kills_process_gracefully() {
    // Arrange
    let node = Node::default();
    let pid = node.spawn(MyProc).await;

    // Act
    let is_alive_before_exit = node.is_alive(&pid);
    node.exit(&pid).await;
    tokio::time::sleep(Duration::from_millis(0)).await;
    let is_alive_after_exit = node.is_alive(&pid);

    // Assert
    assert!(is_alive_before_exit);
    assert!(!is_alive_after_exit);
}

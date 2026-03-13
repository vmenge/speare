mod sync_vec;

use speare::{Actor, Ctx, Node, RegistryError};
use std::time::Duration;
use sync_vec::SyncVec;
use tokio::time;

struct Worker;

impl Actor for Worker {
    type Props = SyncVec<String>;
    type Msg = String;
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Worker)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        ctx.props().push(msg).await;
        Ok(())
    }
}

struct LookupByType;

impl Actor for LookupByType {
    type Props = SyncVec<bool>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let found = ctx.get_handle_for::<Worker>().is_ok();
        ctx.props().push(found).await;
        Ok(LookupByType)
    }
}

struct LookupByName;

impl Actor for LookupByName {
    type Props = (String, SyncVec<bool>);
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let (name, recvd) = ctx.props();
        let found = ctx.get_handle::<String>(name).is_ok();
        recvd.push(found).await;
        Ok(LookupByName)
    }
}

// -- spawn_registered --------------------------------------------------------

#[tokio::test]
async fn spawn_registered_returns_handle() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<String>::default();

    // Act
    let result = node.actor::<Worker>(recvd).spawn_registered();

    // Assert
    assert!(result.is_ok());
}

#[tokio::test]
async fn spawn_registered_returns_name_taken_on_duplicate() {
    // Arrange
    let mut node = Node::default();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_registered()
        .unwrap();

    // Act
    let result = node.actor::<Worker>(SyncVec::default()).spawn_registered();

    // Assert
    assert!(matches!(result, Err(RegistryError::NameTaken(_))));
}

#[tokio::test]
async fn spawn_registered_actor_is_functional() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<String>::default();
    let handle = node
        .actor::<Worker>(recvd.clone())
        .spawn_registered()
        .unwrap();

    // Act
    handle.send("hello".to_string());
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec!["hello".to_string()]);
}

// -- spawn_named -------------------------------------------------------------

#[tokio::test]
async fn spawn_named_returns_handle() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<String>::default();

    // Act
    let result = node.actor::<Worker>(recvd).spawn_named("my-worker");

    // Assert
    assert!(result.is_ok());
}

#[tokio::test]
async fn spawn_named_returns_name_taken_on_duplicate() {
    // Arrange
    let mut node = Node::default();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("my-worker")
        .unwrap();

    // Act
    let result = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("my-worker");

    // Assert
    assert!(matches!(result, Err(RegistryError::NameTaken(_))));
}

#[tokio::test]
async fn spawn_named_allows_different_names() {
    // Arrange
    let mut node = Node::default();

    // Act
    let first = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("worker-a");
    let second = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("worker-b");

    // Assert
    assert!(first.is_ok());
    assert!(second.is_ok());
}

#[tokio::test]
async fn spawn_named_actor_is_functional() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<String>::default();
    let handle = node
        .actor::<Worker>(recvd.clone())
        .spawn_named("worker")
        .unwrap();

    // Act
    handle.send("world".to_string());
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec!["world".to_string()]);
}

// -- get_handle_for ----------------------------------------------------------

#[tokio::test]
async fn get_handle_for_returns_none_when_not_registered() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();

    // Act - LookupByType tries to find Worker, which was never registered
    node.actor::<LookupByType>(recvd.clone()).spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![false]);
}

#[tokio::test]
async fn get_handle_for_finds_registered_actor() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_registered()
        .unwrap();

    // Act - LookupByType tries to find Worker, which was registered above
    node.actor::<LookupByType>(recvd.clone()).spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![true]);
}

// -- get_handle (type mismatch) ----------------------------------------------

struct LookupByNameWrongType;

impl Actor for LookupByNameWrongType {
    type Props = (String, SyncVec<bool>);
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let (name, recvd) = ctx.props();
        let found = ctx.get_handle::<u32>(name).is_ok();
        recvd.push(found).await;
        Ok(LookupByNameWrongType)
    }
}

#[tokio::test]
async fn get_handle_returns_none_for_wrong_msg_type() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("my-worker")
        .unwrap();

    // Act - lookup with u32 instead of String (Worker::Msg = String)
    node.actor::<LookupByNameWrongType>(("my-worker".to_string(), recvd.clone()))
        .spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![false]);
}

// -- get_handle --------------------------------------------------------------

#[tokio::test]
async fn get_handle_returns_none_when_not_registered() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();

    // Act
    node.actor::<LookupByName>(("nonexistent".to_string(), recvd.clone()))
        .spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![false]);
}

#[tokio::test]
async fn get_handle_finds_named_actor() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("my-worker")
        .unwrap();

    // Act
    node.actor::<LookupByName>(("my-worker".to_string(), recvd.clone()))
        .spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![true]);
}

#[tokio::test]
async fn get_handle_returns_none_for_wrong_name() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("my-worker")
        .unwrap();

    // Act
    node.actor::<LookupByName>(("wrong-name".to_string(), recvd.clone()))
        .spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![false]);
}

// -- send (by type) ----------------------------------------------------------

struct SendByType;

impl Actor for SendByType {
    type Props = ();
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let _ = ctx.send::<Worker>("via-send".to_string());
        Ok(SendByType)
    }
}

#[tokio::test]
async fn send_delivers_msg_to_registered_actor() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<String>::default();
    let _ = node
        .actor::<Worker>(recvd.clone())
        .spawn_registered()
        .unwrap();

    // Act
    node.actor::<SendByType>(()).spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec!["via-send".to_string()]);
}

struct SendByTypeNotFound;

impl Actor for SendByTypeNotFound {
    type Props = SyncVec<bool>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let result = ctx.send::<Worker>("msg".to_string());
        ctx.props()
            .push(matches!(result, Err(RegistryError::NotFound(_))))
            .await;

        Ok(SendByTypeNotFound)
    }
}

#[tokio::test]
async fn send_returns_not_found_when_not_registered() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();

    // Act
    node.actor::<SendByTypeNotFound>(recvd.clone()).spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![true]);
}

// -- send_to (by name) -------------------------------------------------------

struct SendByName;

impl Actor for SendByName {
    type Props = String;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let name = ctx.props().clone();
        let _ = ctx.send_to::<<Worker as Actor>::Msg>(&name, "via-send-to".to_string());

        Ok(SendByName)
    }
}

#[tokio::test]
async fn send_to_delivers_msg_to_named_actor() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<String>::default();
    let _ = node
        .actor::<Worker>(recvd.clone())
        .spawn_named("target")
        .unwrap();

    // Act
    node.actor::<SendByName>("target".to_string()).spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec!["via-send-to".to_string()]);
}

struct SendByNameNotFound;

impl Actor for SendByNameNotFound {
    type Props = SyncVec<bool>;
    type Msg = ();
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let result = ctx.send_to::<<Worker as Actor>::Msg>("nope", "msg".to_string());
        ctx.props()
            .push(matches!(result, Err(RegistryError::NotFound(_))))
            .await;

        Ok(SendByNameNotFound)
    }
}

#[tokio::test]
async fn send_to_returns_not_found_when_not_registered() {
    // Arrange
    let mut node = Node::default();
    let recvd = SyncVec::<bool>::default();

    // Act
    node.actor::<SendByNameNotFound>(recvd.clone()).spawn();
    time::sleep(Duration::from_millis(10)).await;

    // Assert
    assert_eq!(recvd.clone_vec().await, vec![true]);
}

// -- deregistration -----------------------------------------------------------

#[tokio::test]
async fn named_actor_deregisters_on_stop() {
    // Arrange
    let mut node = Node::default();
    let handle = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("worker")
        .unwrap();

    // Act
    handle.stop();
    time::sleep(Duration::from_millis(50)).await;

    // Assert - name should be available again
    let result = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("worker");
    assert!(result.is_ok());
}

#[tokio::test]
async fn registered_actor_deregisters_on_stop() {
    // Arrange
    let mut node = Node::default();
    let handle = node
        .actor::<Worker>(SyncVec::default())
        .spawn_registered()
        .unwrap();

    // Act
    handle.stop();
    time::sleep(Duration::from_millis(50)).await;

    // Assert - type name should be available again
    let result = node.actor::<Worker>(SyncVec::default()).spawn_registered();
    assert!(result.is_ok());
}

// -- spawn_registered / spawn_named collision ---------------------------------

#[tokio::test]
async fn spawn_named_with_type_name_collides_with_spawn_registered() {
    // Arrange
    let mut node = Node::default();
    let type_name = std::any::type_name::<Worker>();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named(type_name)
        .unwrap();

    // Act
    let result = node.actor::<Worker>(SyncVec::default()).spawn_registered();

    // Assert
    assert!(matches!(result, Err(RegistryError::NameTaken(_))));
}

#[tokio::test]
async fn spawn_registered_collides_with_spawn_named_using_type_name() {
    // Arrange
    let mut node = Node::default();
    let _ = node
        .actor::<Worker>(SyncVec::default())
        .spawn_registered()
        .unwrap();

    // Act
    let type_name = std::any::type_name::<Worker>();
    let result = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named(type_name);

    // Assert
    assert!(matches!(result, Err(RegistryError::NameTaken(_))));
}

#[tokio::test]
async fn spawn_registered_and_spawn_named_coexist_with_different_keys() {
    // Arrange & Act
    let mut node = Node::default();
    let first = node.actor::<Worker>(SyncVec::default()).spawn_registered();

    let second = node
        .actor::<Worker>(SyncVec::default())
        .spawn_named("custom-worker");

    // Assert
    assert!(first.is_ok());
    assert!(second.is_ok());
}

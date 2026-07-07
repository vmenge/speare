use speare::prelude::*;

#[test]
fn get_returns_inserted_resource() -> Result<()> {
    // Arrange
    let mut app = App::new();
    app.insert_resource(5i32);

    // Act
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 5);
    Ok(())
}

#[test]
fn override_resource_replaces_existing_resource() -> Result<()> {
    // Arrange
    let mut app = App::new();
    app.insert_resource(5i32);

    // Act
    app.override_resource(9i32);
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 9);
    Ok(())
}

#[tokio::test]
async fn spawn_join_returns_system_error() {
    let err = App::new()
        .add_system(Schedule::Start, || async {
            Err(Error::source(std::io::Error::other("failed")))
        })
        .spawn()
        .join()
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn app_can_be_accessed_after_spawn() -> Result<()> {
    let mut app = App::new();
    app.insert_resource(5i32);

    let handle = app.spawn();
    assert_eq!(*app.get::<i32>()?, 5);

    handle.join().await
}

#[tokio::test]
async fn run_executes_one_system_with_app_resources() -> Result<()> {
    let mut app = App::new();
    app.insert_resource(5i32);

    app.run(|value: Res<i32>| async move {
        assert_eq!(*value, 5);
    })
    .await?;

    Ok(())
}

#[tokio::test]
async fn run_returns_extraction_error() {
    let err = App::new()
        .run(|_: Res<i32>| async {})
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "i32 is not registered");
}

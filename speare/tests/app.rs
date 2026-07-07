use speare::prelude::*;

struct InsertI32Plugin(i32);

impl Plugin for InsertI32Plugin {
    fn setup(&self, builder: &mut AppBuilder) {
        builder.insert_resource(self.0);
    }
}

#[test]
fn builder_builds_app_with_inserted_resource() -> Result<()> {
    // Arrange
    let app = App::builder().insert_resource(5i32).build();

    // Act
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 5);
    Ok(())
}

#[test]
fn builder_override_resource_replaces_existing_resource() -> Result<()> {
    // Arrange
    let app = App::builder()
        .insert_resource(5i32)
        .override_resource(9i32)
        .build();

    // Act
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 9);
    Ok(())
}

#[test]
fn app_new_builds_empty_runtime_app() {
    // Arrange
    let app = App::new();

    // Act
    let err = app.get::<i32>().unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "i32 is not registered");
}

#[test]
fn builder_add_plugin_runs_setup_before_build() -> Result<()> {
    // Arrange
    let plugin = InsertI32Plugin(42);

    // Act
    let app = App::builder().add_plugin(plugin).build();
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 42);
    Ok(())
}

#[tokio::test]
async fn spawn_join_returns_task_error() {
    // Arrange / Act
    let err = App::new()
        .add_task(Schedule::Start, || async {
            Err(Error::source(std::io::Error::other("failed")))
        })
        .spawn()
        .join()
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn app_can_be_accessed_after_spawn() -> Result<()> {
    // Arrange
    let app = App::builder().insert_resource(5i32).build();

    // Act
    let handle = app.spawn();
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 5);
    handle.join().await
}

#[tokio::test]
async fn run_executes_one_task_with_app_resources() -> Result<()> {
    // Arrange
    let app = App::builder().insert_resource(5i32).build();

    // Act
    app.run(|value: Res<i32>| async move {
        // Assert
        assert_eq!(*value, 5);
    })
    .await?;

    Ok(())
}

#[tokio::test]
async fn run_returns_extraction_error() {
    // Arrange / Act
    let err = App::new()
        .run(|_: Res<i32>| async {})
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "i32 is not registered");
}

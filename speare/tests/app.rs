use speare::Error;
use speare::prelude::*;

struct InsertI32Plugin(i32);

impl Plugin for InsertI32Plugin {
    fn setup(&self, builder: &mut AppBuilder) {
        builder.insert_resource(self.0);
    }
}

#[test]
fn builder_builds_app_with_inserted_resource() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(5i32).build();

    // Act
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 5);
    Ok(())
}

#[test]
fn builder_override_resource_replaces_existing_resource() -> Result<(), Error> {
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
fn builder_add_plugin_runs_setup_before_build() -> Result<(), Error> {
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
    let mut app = App::new();
    app.spawn(async || -> Result<(), String> { Err("failed".into()) });

    let err = app.join().await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "failed");
}

#[tokio::test]
async fn app_can_be_accessed_after_spawn() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder().insert_resource(5i32).build();

    // Act
    app.spawn(|| async {});
    let value = app.get::<i32>()?;

    // Assert
    assert_eq!(*value, 5);
    app.join().await
}

#[tokio::test]
async fn run_executes_one_task_with_app_resources() -> Result<(), Error> {
    // Arrange
    let app = App::builder().insert_resource(5i32).build();

    // Act
    app.run(async |value: Res<i32>| {
        // Assert
        assert_eq!(*value, 5);
    })
    .await?;

    Ok(())
}

#[tokio::test]
async fn run_returns_extraction_error() {
    // Arrange / Act
    let err = App::new().run(async |_: Res<i32>| {}).await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "i32 is not registered");
}

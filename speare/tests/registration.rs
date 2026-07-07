use speare::Error;
use speare::prelude::*;

#[tokio::test]
async fn builder_channel_registration_provides_sender_and_receiver() -> Result<(), Error> {
    // Arrange
    let app = App::builder().register_channel::<i32>().build();

    // Act
    app.run(
        async |sender: Sender<i32>, receiver: Receiver<i32>| -> Result<(), Error> {
            sender.send(7).await?;
            let message = receiver.recv().await?;

            // Assert
            assert_eq!(message, 7);

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn builder_broadcast_registration_provides_publisher_and_subscriber() -> Result<(), Error> {
    // Arrange
    let app = App::builder().register_broadcast::<i32>(4).build();

    // Act
    app.run(
        async |publisher: Pub<i32>, mut subscriber: Sub<i32>| -> Result<(), Error> {
            publisher.send(11)?;
            let message = subscriber.recv().await?;

            // Assert
            assert_eq!(message, 11);

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn builder_query_registration_provides_querier_and_query() -> Result<(), Error> {
    // Arrange
    let mut app = App::builder().register_query::<String, usize>().build();

    // Act
    app.spawn(async |query: Query<String, usize>| -> Result<(), Error> {
        let request = query.recv().await?;
        let response = request.data.len();
        request.reply(response)?;

        Ok(())
    })
    .run(
        async |querier: Querier<String, usize>| -> Result<(), Error> {
            let response = querier.query("speare".to_owned()).await?;

            // Assert
            assert_eq!(response, 6);

            Ok(())
        },
    )
    .await
}

#[tokio::test]
async fn missing_channel_registration_fails_during_task_param_extraction() {
    // Arrange
    let app = App::builder().register_channel::<u64>().build();

    // Act
    let err = app.run(async |_: Sender<i32>| {}).await.unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "i32 is not registered as a channel");
}

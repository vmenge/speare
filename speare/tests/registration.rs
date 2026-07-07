use speare::prelude::*;

#[tokio::test]
async fn builder_channel_registration_provides_sender_and_receiver() -> Result<()> {
    // Arrange
    let app = App::builder().register_channel::<i32>().build();

    // Act
    app.run(|sender: Sender<i32>, receiver: Receiver<i32>| async move {
        sender.send(7).await?;
        let message = receiver.recv().await?;

        // Assert
        assert_eq!(message, 7);
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn builder_broadcast_registration_provides_publisher_and_subscriber() -> Result<()> {
    // Arrange
    let app = App::builder().register_broadcast::<i32>(4).build();

    // Act
    app.run(|publisher: Pub<i32>, mut subscriber: Sub<i32>| async move {
        publisher.send(11)?;
        let message = subscriber.recv().await?;

        // Assert
        assert_eq!(message, 11);
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn builder_query_registration_provides_querier_and_query_worker() -> Result<()> {
    // Arrange
    let app = App::builder().register_query::<String, usize>().build();

    // Act
    app.run(
        |querier: Querier<String, usize>, query: Query<String, usize>| async move {
            let client = async {
                let response = querier.query("speare".to_owned()).await?;

                // Assert
                assert_eq!(response, 6);
                Ok::<(), Error>(())
            };

            let worker = async {
                let request = query.recv().await?;
                let response = request.data.len();
                request.reply(response)?;
                Ok::<(), Error>(())
            };

            let (client, worker) = tokio::join!(client, worker);
            client?;
            worker?;
            Ok::<(), Error>(())
        },
    )
    .await
}

#[tokio::test]
async fn missing_channel_registration_fails_during_task_param_extraction() {
    // Arrange
    let app = App::builder().register_channel::<u64>().build();

    // Act
    let err = app
        .run(|_: Sender<i32>| async {})
        .await
        .unwrap_err();

    // Assert
    assert_eq!(err.to_string(), "i32 is not registered as a channel");
}

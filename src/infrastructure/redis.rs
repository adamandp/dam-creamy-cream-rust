use redis::Client;

pub async fn connect(url: &str) -> Result<Client, redis::RedisError> {
    let client = Client::open(url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let response: String = redis::cmd("PING").query_async(&mut conn).await?;
    if response != "PONG" {
        return Err(redis::RedisError::from((
            redis::ErrorKind::ResponseError,
            "unexpected Redis PING response",
        )));
    }
    Ok(client)
}

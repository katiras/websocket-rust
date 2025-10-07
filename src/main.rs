use log::LevelFilter;
use std::error::Error;

mod chat_message;
mod hub;
mod server;

use hub::MessageHub;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::Builder::from_default_env()
        .filter_level(LevelFilter::Debug)
        .init();

    let hub = MessageHub::new();
    server::run("127.0.0.1:8080", hub).await
}

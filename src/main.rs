use log::{LevelFilter, info};
use tokio::net::TcpListener;
mod connection;
mod error;
mod message;
mod message_hub;
use crate::{connection::handle_connection, message_hub::MessageHub};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(LevelFilter::Debug)
        .init();

    let addr = "127.0.0.1:8080";
    let listener = TcpListener::bind(addr).await.expect("Failed to bind");
    let msg_hub = MessageHub::new();

    info!("WebSocket server listening on ws://{}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let msg_hub_clone = msg_hub.clone();
        tokio::spawn(handle_connection(stream, msg_hub_clone));
    }
}

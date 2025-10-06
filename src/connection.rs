use futures_util::{SinkExt, StreamExt};
use log::info;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};
use uuid::Uuid;

use crate::error::DynError;
use crate::message::MessageHandler;
use crate::message_hub::MessageHub;

pub async fn handle_connection(stream: TcpStream, msg_hub: MessageHub) -> Result<(), DynError> {
    let peer_addr = stream.peer_addr()?;
    let client_id = Uuid::new_v4();

    info!("New connection from {} assigned id {}", peer_addr, client_id);

    let ws_stream = accept_async(stream).await?;

    let (tx, rx) = mpsc::channel::<Message>(100);

    msg_hub.subscribe(client_id, tx).await;

    let (mut ws_writer, ws_reader) = ws_stream.split();

    let welcome_message = Message::Text(format!("Welcome! your id: {}", client_id).into());
    ws_writer.send(welcome_message).await?;

    let write_handle = spawn_ws_write_task(ws_writer, rx, client_id);
    let read_handle = spawn_ws_read_task(ws_reader, client_id, msg_hub.clone());

    tokio::select! {
        _ = write_handle => {
            info!("Write task finished first for {}", client_id);
        }
        _ = read_handle => {
            info!("Read task finished first for {}", client_id);
        }
    }
    msg_hub.unsubscribe(client_id).await;

    info!("Connection {} ({}) closed", client_id, peer_addr);

    Ok(())
}

fn spawn_ws_write_task(
    mut write: futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    mut rx: mpsc::Receiver<Message>,
    client_id: Uuid,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = write.send(msg).await {
                info!("Write error for client {}: {}", client_id, e);
                break;
            }
        }
        info!("Write task ended for client {}", client_id);
    })
}

fn spawn_ws_read_task(
    mut read: futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    client_id: Uuid,
    msg_hub: MessageHub,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let handler = MessageHandler::new(msg_hub);

        while let Some(result) = read.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    if let Err(e) = handler.handle_text_message(client_id, text.to_string()).await {
                        info!("Message handling error for client {}: {}", client_id, e);
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("Client {} sent close frame", client_id);
                    break;
                }
                Ok(_) => {
                    info!("Received non-text message from {}", client_id);
                }
                Err(e) => {
                    info!("Read error for client {}: {}", client_id, e);
                    break;
                }
            }
        }
        info!("Read task ended for client {}", client_id);
    })
}

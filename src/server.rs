use crate::{chat_message::ChatMessage, hub::Hub};
use futures_util::{SinkExt, StreamExt};
use log::info;
use std::error::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};
use uuid::Uuid;

pub async fn run(addr: &str, hub: impl Hub + 'static) -> Result<(), Box<dyn Error + Send + Sync>> {
    let listener = TcpListener::bind(addr).await?;
    info!("WebSocket server listening on ws://{}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let hub = hub.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, hub).await {
                info!("Connection error: {}", e);
            }
        });
    }

    Ok(())
}

async fn handle_connection(stream: TcpStream, hub: impl Hub + 'static) -> Result<(), Box<dyn Error + Send + Sync>> {
    let peer_addr = stream.peer_addr()?;
    let client_id = Uuid::new_v4();

    info!("New connection from {} assigned id {}", peer_addr, client_id);

    let ws_stream = accept_async(stream).await?;
    let (tx, rx) = mpsc::channel::<Message>(100);

    hub.register(client_id, tx).await;

    let (mut ws_writer, ws_reader) = ws_stream.split();

    let welcome_message = Message::Text(format!("Welcome! your id: {}", client_id).into());
    ws_writer.send(welcome_message).await?;

    let hub_clone = hub.clone();
    let write_handle = spawn_write_task(ws_writer, rx, client_id);
    let read_handle = spawn_read_task(ws_reader, client_id, hub_clone);

    tokio::select! {
        _ = write_handle => { info!("Write task finished first for {}", client_id); }
        _ = read_handle => { info!("Read task finished first for {}", client_id); }
    }

    hub.unregister(client_id).await;
    info!("Connection {} ({}) closed", client_id, peer_addr);

    Ok(())
}

fn spawn_write_task(
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

fn spawn_read_task<H: Hub + 'static>(
    mut read: futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    client_id: Uuid,
    hub: H,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(result) = read.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    if let Err(e) = handle_message(client_id, &text, &hub).await {
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

async fn handle_message<H: Hub>(sender_id: Uuid, text: &str, hub: &H) -> Result<(), Box<dyn Error>> {
    let msg = ChatMessage::parse(text)?;
    let receiver_id = msg.receiver_uuid()?;
    hub.send_to(sender_id, receiver_id, msg.text).await;
    Ok(())
}

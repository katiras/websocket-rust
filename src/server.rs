use crate::{chat_message::ChatMessage, hub::Hub};
use futures_util::{SinkExt, StreamExt};
use log::info;
use std::error::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};

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

    let (mut ws_writer, mut ws_reader) = accept_async(stream).await?.split();

    let (tx, rx) = mpsc::channel::<Message>(100);

    let first_msg = match ws_reader.next().await {
        Some(Ok(Message::Text(text))) => text,
        _ => {
            info!("Client {} did not send text as first message", peer_addr);
            return Err("first_msg".into());
        }
    };

    let username = validate_connect_message(&first_msg).await?;
    let client_ids = hub.get_client_ids().await;

    hub.register(username.clone(), tx).await;

    let message = serde_json::json!({
        "type": "userList",
        "users": client_ids
    });
    let msg = Message::Text(message.to_string().into());
    let _ = ws_writer.send(msg).await;

    info!("Client {} connected as {}", peer_addr, username);

    let welcome_message = Message::Text(format!("Welcome! your id: {}", username).into());
    ws_writer.send(welcome_message).await?;

    let hub_clone = hub.clone();
    let write_handle = spawn_write_task(ws_writer, rx, username.clone());
    let read_handle = spawn_read_task(ws_reader, username.clone(), hub_clone);

    let _ = tokio::try_join!(write_handle, read_handle);

    hub.unregister(username.clone()).await;
    info!("Connection {} ({}) closed", username, peer_addr);

    Ok(())
}

fn spawn_write_task(
    mut write_ws_stream: futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    mut rx: mpsc::Receiver<Message>,
    username: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = write_ws_stream.send(msg).await {
                info!("Write error for client {}: {}", username, e);
                break;
            }
        }
        info!("Write task ended for client {}", username);
    })
}

fn spawn_read_task<H: Hub + 'static>(
    mut read_ws_stream: futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    client_id: String,
    hub: H,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(result) = read_ws_stream.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    if let Err(e) = handle_message(client_id.clone(), &text, &hub).await {
                        info!("Message error for {}: {}", client_id, e);
                    }
                }
                Ok(Message::Close(_)) => {
                    hub.unregister(client_id.clone()).await;
                    break;
                }
                Err(e) => {
                    info!("Read error for {}: {}", client_id, e);
                    break;
                }
                _ => {}
            }
        }
        info!("Read task ended for {}", client_id);
    })
}

async fn validate_connect_message(msg: &str) -> Result<String, String> {
    let json = serde_json::from_str::<serde_json::Value>(msg).map_err(|e| format!("Invalid JSON: {}", e))?;

    if json.get("type").and_then(|t| t.as_str()) != Some("connect") {
        return Err("First message must be type 'connect'".to_string());
    }

    let username = json
        .get("username")
        .and_then(|u| u.as_str())
        .ok_or("Missing username")?
        .to_string();

    info!("Username: {}", username);

    Ok(username)
}

async fn handle_message<H: Hub>(sender_id: String, text: &str, hub: &H) -> Result<(), Box<dyn Error>> {
    let msg = ChatMessage::parse(text)?;
    let receiver_id = msg.receiver_username();
    hub.send_to(sender_id, receiver_id, msg.text).await;
    Ok(())
}

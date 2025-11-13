use crate::dispatcher::{ClientId, Dispatcher, DispatcherMessage};
use futures_util::{SinkExt, StreamExt};
use log::{LevelFilter, error, info};
use serde::{Deserialize, Serialize};
use std::{error::Error, net::SocketAddr};
use tokio::{
    net::{TcpListener, TcpStream},
    signal,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
mod dispatcher;

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
enum IncomingMessage {
    DirectMessage(IncomingDirectMessageData),
    Connect(ConnectData),
}

#[derive(Debug, Deserialize, Serialize)]
struct IncomingDirectMessageData {
    to: ClientId,
    text: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ConnectData {
    username: ClientId,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
enum OutgoingMessage {
    DirectMessage(OutgoingDirectMessageData),
    UserList(OutgoingUserListData),
    Shutdown,
}

#[derive(Debug, Deserialize, Serialize)]
struct OutgoingUserListData {
    users: Vec<ClientId>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OutgoingDirectMessageData {
    from: ClientId,
    text: String,
}

#[derive(Deserialize, Serialize)]
enum ClientMessage {
    DirectMessage { from: ClientId, text: String },
    UserList { users: Vec<ClientId> },
    Shutdown,
}

const LOGLEVEL: LevelFilter = LevelFilter::Info;
const DISPATCHER_CHANNEL_BUFFER_SIZE: usize = 100;
const CLIENT_CHANNEL_CAPACITY: usize = 32;
const LISTEN_ADDR: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::Builder::from_default_env().filter_level(LOGLEVEL).init();

    let ct = CancellationToken::new();

    let ct_clone = ct.clone();
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                println!("Received Ctrl+C, shutting down...");
                ct_clone.cancel();
            }
            Err(err) => {
                eprintln!("Unable to listen for shutdown signal: {}", err);
            }
        }
    });

    let (disp_tx, disp_rx) = mpsc::channel(DISPATCHER_CHANNEL_BUFFER_SIZE);

    tokio::spawn(Dispatcher::new(disp_rx).run(ct.clone()));

    let listener = TcpListener::bind(LISTEN_ADDR).await?;
    let connection_tasks_tracker = TaskTracker::new();

    loop {
        tokio::select! {
            Ok((tcp_stream, addr)) = listener.accept() => {
                let disp_tx = disp_tx.clone();
                connection_tasks_tracker.spawn(async move {
                    match accept_async(tcp_stream).await {
                        Ok(ws_stream) => {
                            if let Err(e) = handle_connection(ws_stream, disp_tx, addr).await {
                                error!("{}", e);
                            }
                        }
                        Err(e) => error!("WebSocket handshake failed for client {}: {}", addr, e),
                    }
                });
            }
            _ = ct.cancelled() => {
                info!("Shutdown signal received, stopping accepting new connections");
                break;
            }
        }
    }

    connection_tasks_tracker.close();
    connection_tasks_tracker.wait().await;
    info!("All connections closed");

    Ok(())
}

async fn handle_connection(
    ws_stream: WebSocketStream<TcpStream>,
    disp_tx: mpsc::Sender<DispatcherMessage>,
    peer_addr: SocketAddr,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut ws_sender, mut ws_recv) = ws_stream.split();
    let (cl_sender, mut cl_recv) = mpsc::channel::<ClientMessage>(CLIENT_CHANNEL_CAPACITY);

    let first_msg = match ws_recv.next().await {
        Some(Ok(Message::Text(text))) => text,
        _ => {
            return Err(format!("Client {} did not send text as first message", peer_addr).into());
        }
    };

    let client_id = match serde_json::from_str::<IncomingMessage>(&first_msg) {
        Ok(IncomingMessage::Connect(data)) => data.username,
        _ => return Err("Invalid first msg".into()),
    };

    // Register
    let (res_chan_tx, res_chan_rx) = oneshot::channel();
    let register_msg = DispatcherMessage::Register {
        id: client_id.clone(),
        tx: cl_sender,
        res_chan: res_chan_tx,
    };

    disp_tx.send(register_msg).await?;

    res_chan_rx.await.unwrap()?;

    let disp_tx_send_clone = disp_tx.clone();
    let client_id_send_clone = client_id.clone();
    let mut send_task = tokio::spawn(async move {
        while let Some(cl_msg) = cl_recv.recv().await {
            let outgoing_msg = match cl_msg {
                ClientMessage::DirectMessage { from, text } => OutgoingMessage::DirectMessage(OutgoingDirectMessageData { from, text }),
                ClientMessage::UserList { users } => OutgoingMessage::UserList(OutgoingUserListData { users }),
                ClientMessage::Shutdown => OutgoingMessage::Shutdown,
            };
            
            let msg_json_str = serde_json::to_string(&outgoing_msg).unwrap();
            let ws_msg = Message::Text(msg_json_str.into());

            if ws_sender.send(ws_msg).await.is_err() {
                error!("WebSocket send failed for client {}", client_id_send_clone);
                // Unregister
                let unregister_msg = DispatcherMessage::Unregister { id: client_id_send_clone };
                let _ = disp_tx_send_clone.send(unregister_msg).await;
                break;
            }
        }
    });

    let disp_tx_recv_clone = disp_tx.clone();
    let client_id_recv_clone = client_id.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(ws_msg)) = ws_recv.next().await {
            match ws_msg {
                Message::Text(text) => match serde_json::from_str::<IncomingMessage>(&text) {
                    Ok(parsed) => match parsed {
                        IncomingMessage::DirectMessage(data) => {
                            let dm_msg = DispatcherMessage::DirectMessage {
                                from: client_id_recv_clone.clone(),
                                to: data.to,
                                text: data.text,
                            };

                            disp_tx.send(dm_msg).await.unwrap();
                        }
                        _ => error!("Unable to handle incoming message {}", &text),
                    },
                    _ => error!("Failed to deserialize incoming ws message: {}", &text),
                },
                Message::Close(frame) => {
                    info!("{:?}", frame);
                    let unregister_msg = DispatcherMessage::Unregister { id: client_id.clone() };
                    disp_tx_recv_clone.send(unregister_msg).await.unwrap();
                },
                m => error!("Unsupported ws message: {:?}", m),
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
            let _ = recv_task.await;
        },
        _ = &mut recv_task => {
            send_task.abort();
            let _ = send_task.await;
        },
    }
    
    Ok(())
}

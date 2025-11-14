use crate::dispatcher::{ClientId, Dispatcher, DispatcherMessage};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};
use tokio::{
    net::{TcpListener, TcpStream},
    signal,
    sync::mpsc,
};
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info};
mod dispatcher;

const TRACING_LEVEL: tracing::Level = tracing::Level::INFO;
const DISPATCHER_CHANNEL_BUFFER_SIZE: usize = 100;
const CLIENT_CHANNEL_CAPACITY: usize = 32;
const IP: &str = "127.0.0.1";
const PORT: u16 = 8080;

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

#[derive(Deserialize, Serialize, Debug)]
enum ClientMessage {
    DirectMessage { from: ClientId, text: String },
    UserList { users: Vec<ClientId> },
    Accept,
    Reject { reason: String },
    Shutdown,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt().with_max_level(TRACING_LEVEL).init();

    let ct = CancellationToken::new();

    let ct_clone = ct.clone();
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("Shutting down...");
                ct_clone.cancel();
            }
            Err(err) => {
                error!("Unable to listen for shutdown signal: {}", err);
            }
        }
    });

    let (disp_tx, disp_rx) = mpsc::channel::<DispatcherMessage>(DISPATCHER_CHANNEL_BUFFER_SIZE);

    tokio::spawn(Dispatcher::new(disp_rx).run(ct.clone()));

    let socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::from_str(IP)?), PORT);
    let listener = TcpListener::bind(socket_addr).await?;
    info!("Listening on {}:{}", IP, PORT);
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
                info!("No longer accepting new connections");
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
    let (ws_sender, mut ws_recv) = ws_stream.split();
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
    // let (res_chan_tx, res_chan_rx) = oneshot::channel();
    let register_msg = DispatcherMessage::Register {
        id: client_id.clone(),
        tx: cl_sender,
        // res_chan: res_chan_tx,
    };

    disp_tx.send(register_msg).await?;

    match cl_recv.recv().await {
        Some(ClientMessage::Accept) => {}
        Some(ClientMessage::Reject { reason: error }) => return Err(error.into()),
        Some(m) => return Err(format!("Registration failed1 {:?}", m).into()),
        None => return Err("Registration failed2".into()),
    }

    // res_chan_rx.await.unwrap()?;

    let mut send_task = tokio::spawn(cl_recv_handler(ws_sender, cl_recv, disp_tx.clone(), client_id.clone()));
    let mut recv_task = tokio::spawn(ws_recv_handler(ws_recv, disp_tx.clone(), client_id.clone()));

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

    let _ = disp_tx.send(DispatcherMessage::Unregister { id: client_id }).await;

    Ok(())
}

async fn cl_recv_handler(
    mut ws_sender: SplitSink<WebSocketStream<TcpStream>, Message>,
    mut cl_recv: mpsc::Receiver<ClientMessage>,
    disp_tx: mpsc::Sender<DispatcherMessage>,
    client_id: String,
) {
    while let Some(cl_msg) = cl_recv.recv().await {
        let outgoing_msg = match cl_msg {
            ClientMessage::DirectMessage { from, text } => OutgoingMessage::DirectMessage(OutgoingDirectMessageData { from, text }),
            ClientMessage::UserList { users } => OutgoingMessage::UserList(OutgoingUserListData { users }),
            ClientMessage::Shutdown => OutgoingMessage::Shutdown,
            _ => {
                error!("Unable to handle client message");
                continue;
            }
        };

        let msg_json_str = serde_json::to_string(&outgoing_msg).unwrap();
        let ws_msg = Message::Text(msg_json_str.into());

        if ws_sender.send(ws_msg).await.is_err() {
            error!("WebSocket send failed for client {}", client_id);
            let unregister_msg = DispatcherMessage::Unregister { id: client_id.clone() };
            let _ = disp_tx.send(unregister_msg).await;
            break;
        }
    }
}

async fn ws_recv_handler(
    mut ws_recv: SplitStream<WebSocketStream<TcpStream>>,
    disp_tx: mpsc::Sender<DispatcherMessage>,
    client_id: String,
) {
    while let Some(Ok(ws_msg)) = ws_recv.next().await {
        match ws_msg {
            Message::Text(text) => match serde_json::from_str::<IncomingMessage>(&text) {
                Ok(ws_msg) => match ws_msg {
                    IncomingMessage::DirectMessage(data) => {
                        let dm_msg = DispatcherMessage::DirectMessage {
                            from: client_id.clone(),
                            to: data.to,
                            text: data.text,
                        };

                        if let Err(e) = disp_tx.send(dm_msg).await {
                            error!("Failed to send message to dispatcher: {}", e);
                            break;
                        }
                    }
                    _ => error!("Unable to handle incoming message {}", &text),
                },
                _ => error!("Failed to deserialize incoming ws message: {}", &text),
            },
            Message::Close(_) => {
                let unregister_msg = DispatcherMessage::Unregister { id: client_id.clone() };
                let _ = disp_tx.send(unregister_msg).await;
                break;
            }
            m => error!("Unsupported ws message: {:?}", m),
        }
    }
}

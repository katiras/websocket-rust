use crate::{
    CLIENT_CHANNEL_CAPACITY,
    dispatcher::{ClientId, DispatcherMessage},
    protocol::{IncomingMessage, OutgoingMessage},
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::{Deserialize, Serialize};
use std::{error::Error, net::SocketAddr, str::FromStr};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
use tracing::error;

pub struct Client {
    addr: SocketAddr,
    ws_tx: SplitSink<WebSocketStream<TcpStream>, Message>,
    ws_rx: SplitStream<WebSocketStream<TcpStream>>,
    disp_tx: mpsc::Sender<DispatcherMessage>,
}

impl Client {
    pub fn new(
        addr: SocketAddr,
        ws_tx: SplitSink<WebSocketStream<TcpStream>, Message>,
        ws_rx: SplitStream<WebSocketStream<TcpStream>>,
        disp_tx: mpsc::Sender<DispatcherMessage>,
    ) -> Self {
        Self {
            addr,
            ws_tx,
            ws_rx,
            disp_tx,
        }
    }

    pub async fn run(mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let Some(Ok(Message::Text(text))) = self.ws_rx.next().await else {
            return Err(format!("Client {} failed to handshake", self.addr).into());
        };

        let Ok(IncomingMessage::Connect { username }) = IncomingMessage::from_str(&text) else {
            return Err("First message must be 'connect'".into());
        };

        let client_id = username.clone();

        let (cl_tx, cl_rx) = mpsc::channel::<ClientMessage>(CLIENT_CHANNEL_CAPACITY);

        let (res_tx, res_rx) = oneshot::channel();
        self.disp_tx
            .send(DispatcherMessage::Register {
                id: client_id.clone(),
                tx: cl_tx,
                res_chan: res_tx,
            })
            .await?;

        res_rx.await??;

        let mut send_task = tokio::spawn(Self::cl_rx_handler(self.ws_tx, cl_rx, self.disp_tx.clone(), client_id.clone()));
        let mut recv_task = tokio::spawn(Self::ws_rx_handler(self.ws_rx, self.disp_tx.clone(), client_id.clone()));

        tokio::select! {
            _ = &mut send_task => {
                recv_task.abort();
            },
            _ = &mut recv_task => {
                send_task.abort();
            },
        }

        let _ = self.disp_tx.send(DispatcherMessage::Unregister { id: client_id }).await;

        Ok(())
    }

    async fn cl_rx_handler(
        mut ws_tx: SplitSink<WebSocketStream<TcpStream>, Message>,
        mut cl_rx: mpsc::Receiver<ClientMessage>,
        disp_tx: mpsc::Sender<DispatcherMessage>,
        client_id: String,
    ) {
        while let Some(cl_msg) = cl_rx.recv().await {
            let outgoing_msg: OutgoingMessage = cl_msg.into();

            let ws_msg = Message::Text(outgoing_msg.to_json().into());

            if let Err(e) = ws_tx.send(ws_msg).await {
                error!("{}", e);
                let unregister_msg = DispatcherMessage::Unregister { id: client_id.clone() };
                let _ = disp_tx.send(unregister_msg).await;
                break;
            }
        }
    }

    async fn ws_rx_handler(
        mut ws_rx: SplitStream<WebSocketStream<TcpStream>>,
        disp_tx: mpsc::Sender<DispatcherMessage>,
        client_id: String,
    ) {
        while let Some(Ok(ws_msg)) = ws_rx.next().await {
            match ws_msg {
                Message::Text(text) => match IncomingMessage::from_str(&text) {
                    Ok(IncomingMessage::DirectMessage { to, text }) => {
                        let disp_msg = DispatcherMessage::DirectMessage {
                            from: client_id.clone(),
                            to: to,
                            text: text,
                        };

                        if let Err(e) = disp_tx.send(disp_msg).await {
                            error!("{}", e);
                            break;
                        }
                    }
                    Ok(inc_msg) => {
                        error!("Invalid incoming message type: {:?}", inc_msg)
                    }
                    Err(e) => error!("{}", e),
                },
                Message::Close(_) => {
                    let unregister_msg = DispatcherMessage::Unregister { id: client_id.clone() };
                    let _ = disp_tx.send(unregister_msg).await;
                    break;
                }
                m => error!("Unsupported ws message type: {:?}", m),
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
pub enum ClientMessage {
    DirectMessage { from: ClientId, text: String },
    UserList { users: Vec<ClientId> },
    Shutdown,
}

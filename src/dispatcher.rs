use log::{error, info};
use std::{collections::HashMap, error::Error};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::ClientMessage;

pub type ClientId = String;

pub enum DispatcherMessage {
    Register {
        id: ClientId,
        tx: mpsc::Sender<ClientMessage>,
        res_chan: oneshot::Sender<Result<(), Box<dyn Error + Send + Sync>>>,
    },
    Unregister {
        id: ClientId,
    },
    DirectMessage {
        from: ClientId,
        to: ClientId,
        text: String,
    },
}

pub struct Dispatcher {
    clients: HashMap<ClientId, mpsc::Sender<ClientMessage>>,
    rx: mpsc::Receiver<DispatcherMessage>,
}

impl Dispatcher {
    pub fn new(rx: mpsc::Receiver<DispatcherMessage>) -> Self {
        Self {
            clients: HashMap::new(),
            rx,
        }
    }

    pub async fn run(mut self, ct: CancellationToken) {
        loop {
            tokio::select! {
                Some(msg) = self.rx.recv() => {
                    self.handle(msg).await;
                },
                _ = ct.cancelled() => {
                    break;
                }
            }
        }

        self.rx.close();
        while let Some(msg) = self.rx.recv().await {
            self.handle(msg).await;
        }

        for (id, tx) in &self.clients {
            let _ = tx.send(ClientMessage::Shutdown).await;
            info!("Sent shutdown notification to client {}", id);
        }

        self.clients.clear();
    }

    async fn handle(&mut self, msg: DispatcherMessage) {
        match msg {
            DispatcherMessage::Register { id, tx, res_chan } => {
                let result = match self.clients.contains_key(&id) {
                    true => Err(format!("Client id {} already exists", id).into()),
                    false => {
                        self.clients.insert(id.clone(), tx);
                        self.broadcast_user_list();
                        info!("Client {} registered", id);
                        Ok(())
                    }
                };

                let _ = res_chan.send(result);
            }
            DispatcherMessage::Unregister { id } => {
                self.clients.remove(&id);
                self.broadcast_user_list();
                info!("Client {} unregistered", id)
            }
            DispatcherMessage::DirectMessage { from, to, text } => match self.clients.get(&to) {
                Some(tx) => {
                    let cl_msg = ClientMessage::DirectMessage {
                        from: from.clone(),
                        text: text.clone(),
                    };

                    match tx.send(cl_msg).await {
                        Ok(_) => info!("Message {} sent from {} to {}", text, from, to),
                        Err(e) => error!("Failed to send message sent from {} to {} with error: {}", from, to, e),
                    }
                }
                None => error!("Client {} is not registered", to),
            },
        }
    }

    fn broadcast_user_list(&self) {
        for (client_id, tx) in &self.clients {
            let all_other_client_ids = self
                .clients
                .keys()
                .filter(|k| *k != client_id)
                .cloned()
                .collect::<Vec<ClientId>>();

            let cl_msg = ClientMessage::UserList {
                users: all_other_client_ids,
            };

            let _ = tx.try_send(cl_msg);
        }
    }
}

use log::info;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

pub trait Hub: Send + Sync + Clone {
    fn register(&self, client_id: Uuid, sender: mpsc::Sender<Message>) -> impl Future<Output = ()> + Send;
    fn unregister(&self, client_id: Uuid) -> impl Future<Output = ()> + Send;
    fn send_to(&self, sender_id: Uuid, receiver_id: Uuid, text: String) -> impl Future<Output = ()> + Send;
}

#[derive(Clone)]
pub struct MessageHub {
    clients: Arc<RwLock<HashMap<Uuid, mpsc::Sender<Message>>>>,
}

impl MessageHub {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Hub for MessageHub {
    async fn register(&self, client_id: Uuid, sender: mpsc::Sender<Message>) {
        let mut clients = self.clients.write().await;
        clients.insert(client_id, sender);
        info!("Client {} registered", client_id);
    }

    async fn unregister(&self, client_id: Uuid) {
        let mut clients = self.clients.write().await;
        clients.remove(&client_id);
        info!("Client {} unregistered", client_id);
    }

    async fn send_to(&self, sender_id: Uuid, receiver_id: Uuid, text: String) {
        let clients = self.clients.read().await;

        if let Some(tx) = clients.get(&receiver_id) {
            let payload = Message::Text(format!("From {}: {}", sender_id, text).into());

            match tx.try_send(payload) {
                Ok(_) => {
                    info!("Message from {} to {} delivered", sender_id, receiver_id);
                }
                Err(e) => {
                    info!("Failed to send message from {} to {}: {}", sender_id, receiver_id, e);
                }
            }
        } else {
            info!("Receiver {} not found", receiver_id);
        }
    }
}

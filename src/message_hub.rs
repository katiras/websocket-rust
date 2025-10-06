use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

#[derive(Clone)]
pub struct MessageHub {
    subscribers: Arc<RwLock<HashMap<Uuid, mpsc::Sender<Message>>>>,
}

impl MessageHub {
    pub fn new() -> Self {
        let subscribers = Arc::new(RwLock::new(HashMap::<Uuid, mpsc::Sender<Message>>::new()));
        Self { subscribers }
    }

    pub async fn subscribe(&self, client_id: Uuid, sender: mpsc::Sender<Message>) {
        let mut locked_subs = self.subscribers.write().await;
        locked_subs.insert(client_id, sender);
        info!("Client {} subscribed", client_id);
    }

    pub async fn unsubscribe(&self, client_id: Uuid) {
        let mut locked_subs = self.subscribers.write().await;
        locked_subs.remove(&client_id);
        info!("Client {} unsubscribed", client_id);
    }

    pub async fn publish(&self, sender_id: Uuid, receiver_id: Uuid, text: String) {
        let subs = self.subscribers.read().await;

        if let Some(tx) = subs.get(&receiver_id) {
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

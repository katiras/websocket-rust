use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message;

pub trait Hub: Send + Sync + Clone {
    fn register(&self, client_id: String, sender: mpsc::Sender<Message>) -> impl Future<Output = ()> + Send;
    fn unregister(&self, client_id: String) -> impl Future<Output = ()> + Send;
    fn send_to(&self, sender_id: String, receiver_id: String, text: String) -> impl Future<Output = ()> + Send;
    fn get_client_ids(&self) -> impl Future<Output = Vec<String>> + Send;
}

#[derive(Clone)]
pub struct MessageHub {
    clients: Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>,
}

impl MessageHub {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn broadcast_user_list(&self) {
        let client_ids = self.get_client_ids().await;
        let locked_clients = self.clients.read().await;

        for (client_id, tx) in locked_clients.iter() {
            let message = serde_json::json!({
                "type": "userList",
                "users": client_ids.iter().filter(|x| *x != client_id).cloned().collect::<Vec<String>>()
            });

            let msg = Message::Text(message.to_string().into());
            let _ = tx.try_send(msg.clone());
        }
    }
}

impl Hub for MessageHub {
    async fn get_client_ids(&self) -> Vec<String> {
        let locked_clients = self.clients.read().await;
        return locked_clients.keys().map(|x| x.to_string()).collect::<Vec<String>>();
    }

    async fn register(&self, client_id: String, sender: mpsc::Sender<Message>) {
        {
            let mut locked_clients = self.clients.write().await;
            locked_clients.insert(client_id.clone(), sender);
        }

        self.broadcast_user_list().await;

        info!("Client {} registered", client_id);
    }

    async fn unregister(&self, client_id: String) {
        {
            let mut locked_clients = self.clients.write().await;
            locked_clients.remove(&client_id);
        }

        self.broadcast_user_list().await;

        info!("Client {} unregistered", client_id);
    }

    async fn send_to(&self, sender_id: String, receiver_id: String, text: String) {
        let clients = self.clients.read().await;

        if let Some(tx) = clients.get(&receiver_id) {
            let message = serde_json::json!({
                "type": "privateMessage",
                "text": text,
                "sender_id": sender_id.to_string()
            });

            let payload = Message::Text(message.to_string().into());

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

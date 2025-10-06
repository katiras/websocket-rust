use crate::message_hub::MessageHub;
use serde::Deserialize;
use std::error::Error;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct Message {
    pub receiver_id: String,
    pub text: String,
}

pub struct MessageHandler {
    msg_hub: MessageHub,
}

impl MessageHandler {
    pub fn new(msg_hub: MessageHub) -> Self {
        Self { msg_hub }
    }

    pub async fn handle_text_message(&self, sender_id: Uuid, text: String) -> Result<(), Box<dyn Error>> {
        let data = serde_json::from_str::<Message>(text.as_str())?;
        let receiver_id = Uuid::parse_str(data.receiver_id.as_str())?;
        self.msg_hub.publish(sender_id, receiver_id, data.text).await;
        Ok(())
    }
}

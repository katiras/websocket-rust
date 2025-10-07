use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ChatMessage {
    pub receiver_id: String,
    pub text: String,
}

impl ChatMessage {
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn receiver_uuid(&self) -> Result<Uuid, uuid::Error> {
        Uuid::parse_str(&self.receiver_id)
    }
}

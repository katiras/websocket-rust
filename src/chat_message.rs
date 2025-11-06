use serde::Deserialize;

#[derive(Deserialize)]
pub struct ChatMessage {
    pub receiver_id: String,
    pub text: String,
}

impl ChatMessage {
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn receiver_username(&self) -> String {
        self.receiver_id.clone()
    }
}

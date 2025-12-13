use crate::{client::ClientMessage, dispatcher::ClientId};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum IncomingMessage {
    DirectMessage { to: ClientId, text: String },
    Connect { username: ClientId },
}

impl FromStr for IncomingMessage {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum OutgoingMessage {
    DirectMessage { from: ClientId, text: String },
    UserList { users: Vec<ClientId> },
    Shutdown,
}

impl OutgoingMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Serialization of internal types should not fail")
    }
}

impl From<ClientMessage> for OutgoingMessage {
    fn from(msg: ClientMessage) -> Self {
        match msg {
            ClientMessage::DirectMessage { from, text } => OutgoingMessage::DirectMessage { from, text },
            ClientMessage::UserList { users } => OutgoingMessage::UserList { users },
            ClientMessage::Shutdown => OutgoingMessage::Shutdown,
        }
    }
}

use super::{
    protocol::WsMessage,
    types::{MessageKind, RequestId, Topic},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionEpoch(pub u64);

#[derive(Debug, Clone)]
pub enum ControlCommand {
    Close,
    Reconnect { reason: String },
}

#[derive(Debug, Clone)]
pub enum DataCommand {
    Subscribe {
        topics: Vec<Topic>,
    },
    Unsubscribe {
        topics: Vec<Topic>,
    },
    Send {
        message: WsMessage,
    },
    Request {
        message: WsMessage,
        request_id: RequestId,
    },
}

#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub raw: WsMessage,
    pub text: Option<String>,
    pub kind: MessageKind,
    pub topic: Option<Topic>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected {
        epoch: ConnectionEpoch,
    },
    Disconnected {
        epoch: ConnectionEpoch,
        reason: String,
    },
    Message(IncomingMessage),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_types_compile() {
        let _epoch = ConnectionEpoch(1);

        let _control = ControlCommand::Close;
        let _data = DataCommand::Subscribe {
            topics: vec![Topic::from("trades.BTC")],
        };

        let message = IncomingMessage {
            raw: WsMessage::text("update"),
            text: Some("update".to_string()),
            kind: MessageKind::Unknown,
            topic: None,
        };

        let _event = Event::Message(message);
    }
}

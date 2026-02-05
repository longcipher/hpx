//! Generic JSON protocol handler with configurable field names.
//!
//! This handler works with many JSON-based WebSocket APIs by allowing
//! configuration of the field names used for request IDs, topics, etc.

use serde_json::Value;

use crate::websocket::{
    protocol::{ProtocolHandler, WsMessage},
    types::{MessageKind, RequestId, Topic},
};

/// Configuration for the generic JSON handler.
#[derive(Clone, Debug)]
pub struct GenericJsonConfig {
    /// Field name for request ID (e.g., "id", "req_id", "request_id").
    pub request_id_field: String,
    /// Field name for topic/channel (e.g., "topic", "channel", "stream").
    pub topic_field: String,
    /// Operation field name (e.g., "op", "method", "action").
    pub operation_field: String,
    /// Subscribe operation value (e.g., "subscribe", "sub").
    pub subscribe_op: String,
    /// Unsubscribe operation value (e.g., "unsubscribe", "unsub").
    pub unsubscribe_op: String,
    /// Ping operation value (None uses WebSocket-level ping).
    pub ping_op: Option<String>,
    /// Pong operation value.
    pub pong_op: Option<String>,
    /// Field that indicates response type (e.g., "type", "op").
    pub type_field: Option<String>,
    /// Value that indicates a response message.
    pub response_type: Option<String>,
    /// Value that indicates an update message.
    pub update_type: Option<String>,
}

impl Default for GenericJsonConfig {
    fn default() -> Self {
        Self {
            request_id_field: "req_id".to_string(),
            topic_field: "topic".to_string(),
            operation_field: "op".to_string(),
            subscribe_op: "subscribe".to_string(),
            unsubscribe_op: "unsubscribe".to_string(),
            ping_op: Some("ping".to_string()),
            pong_op: Some("pong".to_string()),
            type_field: None,
            response_type: None,
            update_type: None,
        }
    }
}

impl GenericJsonConfig {
    /// Create a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the request ID field name.
    pub fn request_id_field(mut self, field: impl Into<String>) -> Self {
        self.request_id_field = field.into();
        self
    }

    /// Set the topic field name.
    pub fn topic_field(mut self, field: impl Into<String>) -> Self {
        self.topic_field = field.into();
        self
    }

    /// Set the operation field name.
    pub fn operation_field(mut self, field: impl Into<String>) -> Self {
        self.operation_field = field.into();
        self
    }

    /// Set the subscribe operation value.
    pub fn subscribe_op(mut self, op: impl Into<String>) -> Self {
        self.subscribe_op = op.into();
        self
    }

    /// Set the unsubscribe operation value.
    pub fn unsubscribe_op(mut self, op: impl Into<String>) -> Self {
        self.unsubscribe_op = op.into();
        self
    }

    /// Set the ping operation (None to use WebSocket-level ping).
    pub fn ping_op(mut self, op: Option<String>) -> Self {
        self.ping_op = op;
        self
    }
}

/// Generic JSON protocol handler.
///
/// This handler works with common JSON-based WebSocket protocols where
/// messages contain fields for request IDs, topics, and operations.
///
/// # Example
///
/// ```rust,ignore
/// use hpx_transport::websocket::handlers::GenericJsonHandler;
///
/// let handler = GenericJsonHandler::new()
///     .with_config(|c| c.request_id_field("id").topic_field("channel"));
/// ```
pub struct GenericJsonHandler {
    config: GenericJsonConfig,
}

impl GenericJsonHandler {
    /// Create a new handler with default configuration.
    pub fn new() -> Self {
        Self {
            config: GenericJsonConfig::default(),
        }
    }

    /// Create a handler with custom configuration.
    pub fn with_config(mut self, f: impl FnOnce(GenericJsonConfig) -> GenericJsonConfig) -> Self {
        self.config = f(self.config);
        self
    }

    /// Parse a JSON message, returning None on parse failure.
    fn parse_json(&self, message: &str) -> Option<Value> {
        serde_json::from_str(message).ok()
    }

    /// Get a string field from a JSON value.
    fn get_string_field<'a>(&self, json: &'a Value, field: &str) -> Option<&'a str> {
        json.get(field).and_then(|v| v.as_str())
    }
}

impl Default for GenericJsonHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolHandler for GenericJsonHandler {
    fn classify_message(&self, message: &str) -> MessageKind {
        let Some(json) = self.parse_json(message) else {
            return MessageKind::Unknown;
        };

        // Check for explicit type field
        if let Some(type_field) = &self.config.type_field
            && let Some(type_value) = self.get_string_field(&json, type_field)
        {
            if let Some(response_type) = &self.config.response_type
                && type_value == response_type
            {
                return MessageKind::Response;
            }
            if let Some(update_type) = &self.config.update_type
                && type_value == update_type
            {
                return MessageKind::Update;
            }
        }

        // Check for operation field
        if let Some(op) = self.get_string_field(&json, &self.config.operation_field) {
            if let Some(ping_op) = &self.config.ping_op
                && op == ping_op
            {
                return MessageKind::System;
            }
            if let Some(pong_op) = &self.config.pong_op
                && op == pong_op
            {
                return MessageKind::System;
            }
        }

        // Has request_id -> Response
        if json.get(&self.config.request_id_field).is_some() {
            return MessageKind::Response;
        }

        // Has topic -> Update
        if json.get(&self.config.topic_field).is_some() {
            return MessageKind::Update;
        }

        MessageKind::Unknown
    }

    fn extract_request_id(&self, message: &str) -> Option<RequestId> {
        let json = self.parse_json(message)?;
        let id = json.get(&self.config.request_id_field)?;

        // Handle both string and number IDs
        let id_str = match id {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };

        Some(RequestId::from(id_str))
    }

    fn extract_topic(&self, message: &str) -> Option<Topic> {
        let json = self.parse_json(message)?;
        let topic = self.get_string_field(&json, &self.config.topic_field)?;
        Some(Topic::new(topic))
    }

    fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let args: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();

        let msg = serde_json::json!({
            self.config.operation_field.clone(): self.config.subscribe_op,
            self.config.request_id_field.clone(): request_id.as_str(),
            "args": args,
        });

        WsMessage::text(msg.to_string())
    }

    fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let args: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();

        let msg = serde_json::json!({
            self.config.operation_field.clone(): self.config.unsubscribe_op,
            self.config.request_id_field.clone(): request_id.as_str(),
            "args": args,
        });

        WsMessage::text(msg.to_string())
    }

    fn build_ping(&self) -> Option<WsMessage> {
        self.config.ping_op.as_ref().map(|op| {
            let msg = serde_json::json!({
                self.config.operation_field.clone(): op,
            });
            WsMessage::text(msg.to_string())
        })
    }

    fn build_pong(&self, _ping_data: &[u8]) -> Option<WsMessage> {
        self.config.pong_op.as_ref().map(|op| {
            let msg = serde_json::json!({
                self.config.operation_field.clone(): op,
            });
            WsMessage::text(msg.to_string())
        })
    }

    fn is_server_ping(&self, message: &str) -> bool {
        let Some(json) = self.parse_json(message) else {
            return false;
        };
        let Some(ping_op) = &self.config.ping_op else {
            return false;
        };
        self.get_string_field(&json, &self.config.operation_field) == Some(ping_op)
    }

    fn is_pong_response(&self, message: &str) -> bool {
        let Some(json) = self.parse_json(message) else {
            return false;
        };
        let Some(pong_op) = &self.config.pong_op else {
            return false;
        };
        self.get_string_field(&json, &self.config.operation_field) == Some(pong_op)
    }

    fn inject_request_id(&self, message: WsMessage, request_id: &RequestId) -> WsMessage {
        let text = match message {
            WsMessage::Text(t) => t,
            WsMessage::Binary(_) => return message,
        };

        let Ok(mut json): Result<Value, _> = serde_json::from_str(&text) else {
            return WsMessage::text(text);
        };

        if let Value::Object(ref mut map) = json {
            map.insert(
                self.config.request_id_field.clone(),
                Value::String(request_id.to_string()),
            );
        }

        WsMessage::text(json.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_response() {
        let handler = GenericJsonHandler::new();
        let msg = r#"{"req_id": "123", "result": "ok"}"#;
        assert_eq!(handler.classify_message(msg), MessageKind::Response);
    }

    #[test]
    fn test_classify_update() {
        let handler = GenericJsonHandler::new();
        let msg = r#"{"topic": "orderbook.BTC", "data": []}"#;
        assert_eq!(handler.classify_message(msg), MessageKind::Update);
    }

    #[test]
    fn test_extract_request_id() {
        let handler = GenericJsonHandler::new();
        let msg = r#"{"req_id": "abc123", "result": "ok"}"#;
        let id = handler.extract_request_id(msg);
        assert!(id.is_some());
        assert_eq!(id.as_ref().map(|i| i.as_str()), Some("abc123"));
    }

    #[test]
    fn test_extract_topic() {
        let handler = GenericJsonHandler::new();
        let msg = r#"{"topic": "trades.ETH", "data": []}"#;
        let topic = handler.extract_topic(msg);
        assert!(topic.is_some());
        assert_eq!(topic.as_ref().map(|t| t.as_str()), Some("trades.ETH"));
    }

    #[test]
    fn test_build_subscribe() {
        let handler = GenericJsonHandler::new();
        let msg = handler.build_subscribe(
            &[Topic::new("orderbook.BTC"), Topic::new("trades.ETH")],
            RequestId::from("req1"),
        );
        let text = msg.as_text();
        assert!(text.is_some());
        assert!(text.map(|t| t.contains("subscribe")).unwrap_or(false));
    }

    #[test]
    fn test_custom_config() {
        let handler = GenericJsonHandler::new()
            .with_config(|c| c.request_id_field("id").topic_field("channel"));

        let msg = r#"{"id": "req1", "result": "ok"}"#;
        assert_eq!(handler.classify_message(msg), MessageKind::Response);

        let msg = r#"{"channel": "btc_orderbook", "data": {}}"#;
        assert_eq!(handler.classify_message(msg), MessageKind::Update);
    }
}

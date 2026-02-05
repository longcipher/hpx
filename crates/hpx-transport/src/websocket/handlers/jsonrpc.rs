//! JSON-RPC 2.0 protocol handler.
//!
//! Implements the JSON-RPC 2.0 specification for WebSocket communication.
//! See: <https://www.jsonrpc.org/specification>

use serde_json::Value;

use crate::websocket::{
    protocol::{ProtocolHandler, WsMessage},
    types::{MessageKind, RequestId, Topic},
};

/// JSON-RPC 2.0 protocol handler.
///
/// Handles messages following the JSON-RPC 2.0 specification:
/// - Requests: `{"jsonrpc": "2.0", "method": "...", "params": ..., "id": ...}`
/// - Responses: `{"jsonrpc": "2.0", "result": ..., "id": ...}`
/// - Notifications: `{"jsonrpc": "2.0", "method": "...", "params": ...}` (no id)
///
/// # Example
///
/// ```rust,ignore
/// use hpx_transport::websocket::handlers::JsonRpcHandler;
///
/// let handler = JsonRpcHandler::new()
///     .subscription_method("subscribe")
///     .unsubscription_method("unsubscribe");
/// ```
pub struct JsonRpcHandler {
    /// Method name for subscriptions.
    subscribe_method: String,
    /// Method name for unsubscriptions.
    unsubscribe_method: String,
    /// Field in notifications that contains the topic/channel.
    topic_field: String,
}

impl JsonRpcHandler {
    /// Create a new JSON-RPC handler with default settings.
    pub fn new() -> Self {
        Self {
            subscribe_method: "subscribe".to_string(),
            unsubscribe_method: "unsubscribe".to_string(),
            topic_field: "subscription".to_string(),
        }
    }

    /// Set the subscription method name.
    pub fn subscription_method(mut self, method: impl Into<String>) -> Self {
        self.subscribe_method = method.into();
        self
    }

    /// Set the unsubscription method name.
    pub fn unsubscription_method(mut self, method: impl Into<String>) -> Self {
        self.unsubscribe_method = method.into();
        self
    }

    /// Set the topic field name in notifications.
    pub fn topic_field(mut self, field: impl Into<String>) -> Self {
        self.topic_field = field.into();
        self
    }

    /// Parse JSON message.
    fn parse_json(&self, message: &str) -> Option<Value> {
        serde_json::from_str(message).ok()
    }

    /// Check if this is a valid JSON-RPC 2.0 message.
    fn is_jsonrpc(&self, json: &Value) -> bool {
        json.get("jsonrpc").and_then(|v| v.as_str()) == Some("2.0")
    }
}

impl Default for JsonRpcHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolHandler for JsonRpcHandler {
    fn classify_message(&self, message: &str) -> MessageKind {
        let Some(json) = self.parse_json(message) else {
            return MessageKind::Unknown;
        };

        if !self.is_jsonrpc(&json) {
            return MessageKind::Unknown;
        }

        // Has "id" and ("result" or "error") -> Response
        if json.get("id").is_some() && (json.get("result").is_some() || json.get("error").is_some())
        {
            return MessageKind::Response;
        }

        // Has "method" but no "id" -> Notification (Update)
        if json.get("method").is_some() && json.get("id").is_none() {
            return MessageKind::Update;
        }

        MessageKind::Control
    }

    fn extract_request_id(&self, message: &str) -> Option<RequestId> {
        let json = self.parse_json(message)?;
        let id = json.get("id")?;

        let id_str = match id {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Null => return None,
            _ => return None,
        };

        Some(RequestId::from(id_str))
    }

    fn extract_topic(&self, message: &str) -> Option<Topic> {
        let json = self.parse_json(message)?;

        // Try params.subscription first
        if let Some(params) = json.get("params")
            && let Some(topic) = params.get(&self.topic_field).and_then(|v| v.as_str())
        {
            return Some(Topic::new(topic));
        }

        // Try method name as topic for notifications
        if let Some(method) = json.get("method").and_then(|v| v.as_str()) {
            return Some(Topic::new(method));
        }

        None
    }

    fn build_subscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let params: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id.as_str(),
            "method": self.subscribe_method,
            "params": params,
        });

        WsMessage::text(msg.to_string())
    }

    fn build_unsubscribe(&self, topics: &[Topic], request_id: RequestId) -> WsMessage {
        let params: Vec<&str> = topics.iter().map(|t| t.as_str()).collect();

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id.as_str(),
            "method": self.unsubscribe_method,
            "params": params,
        });

        WsMessage::text(msg.to_string())
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
            map.insert("id".to_string(), Value::String(request_id.to_string()));
            // Ensure jsonrpc version is set
            map.entry("jsonrpc")
                .or_insert(Value::String("2.0".to_string()));
        }

        WsMessage::text(json.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_response() {
        let handler = JsonRpcHandler::new();
        let msg = r#"{"jsonrpc": "2.0", "id": "1", "result": {"status": "ok"}}"#;
        assert_eq!(handler.classify_message(msg), MessageKind::Response);
    }

    #[test]
    fn test_classify_error_response() {
        let handler = JsonRpcHandler::new();
        let msg = r#"{"jsonrpc": "2.0", "id": "1", "error": {"code": -1, "message": "Error"}}"#;
        assert_eq!(handler.classify_message(msg), MessageKind::Response);
    }

    #[test]
    fn test_classify_notification() {
        let handler = JsonRpcHandler::new();
        let msg = r#"{"jsonrpc": "2.0", "method": "subscription.update", "params": {}}"#;
        assert_eq!(handler.classify_message(msg), MessageKind::Update);
    }

    #[test]
    fn test_extract_request_id() {
        let handler = JsonRpcHandler::new();
        let msg = r#"{"jsonrpc": "2.0", "id": "req123", "result": {}}"#;
        let id = handler.extract_request_id(msg);
        assert_eq!(id.as_ref().map(|i| i.as_str()), Some("req123"));
    }

    #[test]
    fn test_extract_topic_from_method() {
        let handler = JsonRpcHandler::new();
        let msg = r#"{"jsonrpc": "2.0", "method": "orderbook.update", "params": {}}"#;
        let topic = handler.extract_topic(msg);
        assert_eq!(topic.as_ref().map(|t| t.as_str()), Some("orderbook.update"));
    }

    #[test]
    fn test_build_subscribe() {
        let handler = JsonRpcHandler::new();
        let msg = handler.build_subscribe(&[Topic::new("channel1")], RequestId::from("1"));
        let text = msg.as_text();
        assert!(text.is_some());
        let text = text.map(|t| t.to_string()).unwrap_or_default();
        assert!(text.contains("jsonrpc"));
        assert!(text.contains("2.0"));
        assert!(text.contains("subscribe"));
    }
}

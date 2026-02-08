//! WebSocket message types and utilities
//!
//! This module provides WebSocket message types offering an ergonomic API
//! for working with WebSocket communications.

use std::{fmt, ops::Deref};

use bytes::Bytes;
#[cfg(feature = "ws-fastwebsockets")]
use fastwebsockets::{Frame, OpCode, Payload};

use crate::Error;

/// UTF-8 wrapper for String.
///
/// An [Utf8Bytes] is always guaranteed to contain valid UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Utf8Bytes(String);

impl Utf8Bytes {
    /// Creates from a static str.
    #[inline]
    pub fn from_static(str: &'static str) -> Self {
        Self(str.to_string())
    }

    /// Returns as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for Utf8Bytes {
    type Target = str;

    /// ```
    /// use hpx::ws::message::Utf8Bytes;
    ///
    /// /// Example fn that takes a str slice
    /// fn a(s: &str) {}
    ///
    /// let data = Utf8Bytes::from_static("foo123");
    ///
    /// // auto-deref as arg
    /// a(&data);
    ///
    /// // deref to str methods
    /// assert_eq!(data.len(), 6);
    /// ```
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for Utf8Bytes {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<Bytes> for Utf8Bytes {
    type Error = std::str::Utf8Error;

    #[inline]
    fn try_from(bytes: Bytes) -> Result<Self, Self::Error> {
        let s = std::str::from_utf8(&bytes)?.to_string();
        Ok(Self(s))
    }
}

impl TryFrom<Vec<u8>> for Utf8Bytes {
    type Error = std::str::Utf8Error;

    #[inline]
    fn try_from(v: Vec<u8>) -> Result<Self, Self::Error> {
        let s = String::from_utf8(v).map_err(|e| e.utf8_error())?;
        Ok(Self(s))
    }
}

impl From<String> for Utf8Bytes {
    #[inline]
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Utf8Bytes {
    #[inline]
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<&String> for Utf8Bytes {
    #[inline]
    fn from(s: &String) -> Self {
        Self(s.clone())
    }
}

impl From<Utf8Bytes> for Bytes {
    #[inline]
    fn from(val: Utf8Bytes) -> Self {
        Bytes::from(val.0)
    }
}

impl<T> PartialEq<T> for Utf8Bytes
where
    for<'a> &'a str: PartialEq<T>,
{
    /// ```
    /// use hpx::ws::message::Utf8Bytes;
    /// let payload = Utf8Bytes::from_static("foo123");
    /// assert_eq!(payload, "foo123");
    /// assert_eq!(payload, "foo123".to_string());
    /// assert_eq!(payload, &"foo123".to_string());
    /// assert_eq!(payload, std::borrow::Cow::from("foo123"));
    /// ```
    #[inline]
    fn eq(&self, other: &T) -> bool {
        self.as_str() == *other
    }
}

/// Status code used to indicate why an endpoint is closing the WebSocket connection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CloseCode(pub(super) u16);

impl CloseCode {
    //! Constants for [`CloseCode`]s.

    /// Indicates a normal closure, meaning that the purpose for which the connection was
    /// established has been fulfilled.
    pub const NORMAL: CloseCode = CloseCode(1000);

    /// Indicates that an endpoint is "going away", such as a server going down or a browser having
    /// navigated away from a page.
    pub const AWAY: CloseCode = CloseCode(1001);

    /// Indicates that an endpoint is terminating the connection due to a protocol error.
    pub const PROTOCOL: CloseCode = CloseCode(1002);

    /// Indicates that an endpoint is terminating the connection because it has received a type of
    /// data that it cannot accept.
    ///
    /// For example, an endpoint MAY send this if it understands only text data, but receives a
    /// binary message.
    pub const UNSUPPORTED: CloseCode = CloseCode(1003);

    /// Indicates that no status code was included in a closing frame.
    pub const STATUS: CloseCode = CloseCode(1005);

    /// Indicates an abnormal closure.
    pub const ABNORMAL: CloseCode = CloseCode(1006);

    /// Indicates that an endpoint is terminating the connection because it has received data
    /// within a message that was not consistent with the type of the message.
    ///
    /// For example, an endpoint received non-UTF-8 RFC3629 data within a text message.
    pub const INVALID: CloseCode = CloseCode(1007);

    /// Indicates that an endpoint is terminating the connection because it has received a message
    /// that violates its policy.
    ///
    /// This is a generic status code that can be returned when there is
    /// no other more suitable status code (e.g., `UNSUPPORTED` or `SIZE`) or if there is a need to
    /// hide specific details about the policy.
    pub const POLICY: CloseCode = CloseCode(1008);

    /// Indicates that an endpoint is terminating the connection because it has received a message
    /// that is too big for it to process.
    pub const SIZE: CloseCode = CloseCode(1009);

    /// Indicates that an endpoint (client) is terminating the connection because the server
    /// did not respond to extension negotiation correctly.
    ///
    /// Specifically, the client has expected the server to negotiate one or more extension(s),
    /// but the server didn't return them in the response message of the WebSocket handshake.
    /// The list of extensions that are needed should be given as the reason for closing.
    /// Note that this status code is not used by the server,
    /// because it can fail the WebSocket handshake instead.
    pub const EXTENSION: CloseCode = CloseCode(1010);

    /// Indicates that a server is terminating the connection because it encountered an unexpected
    /// condition that prevented it from fulfilling the request.
    pub const ERROR: CloseCode = CloseCode(1011);

    /// Indicates that the server is restarting.
    pub const RESTART: CloseCode = CloseCode(1012);

    /// Indicates that the server is overloaded and the client should either connect to a different
    /// IP (when multiple targets exist), or reconnect to the same IP when a user has performed an
    /// action.
    pub const AGAIN: CloseCode = CloseCode(1013);
}

impl From<CloseCode> for u16 {
    #[inline]
    fn from(code: CloseCode) -> u16 {
        code.0
    }
}

impl From<u16> for CloseCode {
    #[inline]
    fn from(code: u16) -> CloseCode {
        CloseCode(code)
    }
}

/// A struct representing the close command.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CloseFrame {
    /// The reason as a code.
    pub code: CloseCode,
    /// The reason as text string.
    pub reason: Utf8Bytes,
}

/// A WebSocket message.
#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    /// A text WebSocket message
    Text(Utf8Bytes),
    /// A binary WebSocket message
    Binary(Bytes),
    /// A ping message with the specified payload
    Ping(Bytes),
    /// A pong message with the specified payload
    Pong(Bytes),
    /// A close message with the optional close frame.
    Close(Option<CloseFrame>),
}

impl Message {
    /// Converts this `Message` into a `fastwebsockets::Frame`.
    #[cfg(feature = "ws-fastwebsockets")]
    pub(crate) fn into_frame(self) -> Frame<'static> {
        match self {
            Self::Text(text) => Frame::text(Payload::Owned(text.0.into_bytes())),
            Self::Binary(data) => Frame::binary(Payload::Owned(data.to_vec())),
            Self::Ping(data) => Frame::new(true, OpCode::Ping, None, Payload::Owned(data.to_vec())),
            Self::Pong(data) => Frame::new(true, OpCode::Pong, None, Payload::Owned(data.to_vec())),
            Self::Close(Some(close)) => {
                Frame::close(close.code.0, close.reason.as_str().as_bytes())
            }
            Self::Close(None) => Frame::new(true, OpCode::Close, None, Payload::Owned(vec![])),
        }
    }

    /// Converts a `fastwebsockets::Frame` into a `Message`.
    #[cfg(feature = "ws-fastwebsockets")]
    pub(crate) fn from_frame(frame: Frame) -> Self {
        let data = Vec::from(frame.payload);
        match frame.opcode {
            OpCode::Text => {
                let s = String::from_utf8_lossy(&data).to_string();
                Self::Text(Utf8Bytes(s))
            }
            OpCode::Binary => Self::Binary(Bytes::from(data)),
            OpCode::Ping => Self::Ping(Bytes::from(data)),
            OpCode::Pong => Self::Pong(Bytes::from(data)),
            OpCode::Close => {
                if data.len() >= 2 {
                    let code = u16::from_be_bytes([data[0], data[1]]);
                    let reason = String::from_utf8_lossy(&data[2..]).to_string();
                    Self::Close(Some(CloseFrame {
                        code: CloseCode(code),
                        reason: Utf8Bytes(reason),
                    }))
                } else {
                    Self::Close(None)
                }
            }
            OpCode::Continuation => Self::Binary(Bytes::from(data)),
        }
    }

    /// Consume the WebSocket and return it as binary data.
    pub fn into_data(self) -> Bytes {
        match self {
            Self::Text(string) => Bytes::from(string),
            Self::Binary(data) | Self::Ping(data) | Self::Pong(data) => data,
            Self::Close(None) => Bytes::new(),
            Self::Close(Some(frame)) => Bytes::from(frame.reason),
        }
    }

    /// Attempt to consume the WebSocket message and convert it to a Utf8Bytes.
    pub fn into_text(self) -> crate::Result<Utf8Bytes> {
        match self {
            Self::Text(string) => Ok(string),
            Self::Binary(data) | Self::Ping(data) | Self::Pong(data) => {
                Utf8Bytes::try_from(data).map_err(Error::decode)
            }
            Self::Close(None) => Ok(Utf8Bytes::default()),
            Self::Close(Some(frame)) => Ok(frame.reason),
        }
    }

    /// Attempt to get a &str from the WebSocket message,
    /// this will try to convert binary data to utf8.
    pub fn to_text(&self) -> crate::Result<&str> {
        match *self {
            Self::Text(ref string) => Ok(string.as_str()),
            Self::Binary(ref data) | Self::Ping(ref data) | Self::Pong(ref data) => {
                std::str::from_utf8(data).map_err(Error::decode)
            }
            Self::Close(None) => Ok(""),
            Self::Close(Some(ref frame)) => Ok(&frame.reason),
        }
    }
}

impl Message {
    /// Create a new text WebSocket message from a stringable.
    pub fn text<S>(string: S) -> Message
    where
        S: Into<Utf8Bytes>,
    {
        Message::Text(string.into())
    }

    /// Create a new binary WebSocket message by converting to `Bytes`.
    pub fn binary<B>(bin: B) -> Message
    where
        B: Into<Bytes>,
    {
        Message::Binary(bin.into())
    }

    /// Create a new ping WebSocket message by converting to `Bytes`.
    pub fn ping<B>(bin: B) -> Message
    where
        B: Into<Bytes>,
    {
        Message::Ping(bin.into())
    }

    /// Create a new pong WebSocket message by converting to `Bytes`.
    pub fn pong<B>(bin: B) -> Message
    where
        B: Into<Bytes>,
    {
        Message::Pong(bin.into())
    }

    /// Create a new close WebSocket message with an optional close frame.
    pub fn close<C>(close: C) -> Message
    where
        C: Into<Option<CloseFrame>>,
    {
        Message::Close(close.into())
    }
}

impl From<String> for Message {
    fn from(string: String) -> Self {
        Message::Text(string.into())
    }
}

impl<'s> From<&'s str> for Message {
    fn from(string: &'s str) -> Self {
        Message::Text(string.into())
    }
}

impl<'b> From<&'b [u8]> for Message {
    fn from(data: &'b [u8]) -> Self {
        Message::Binary(Bytes::copy_from_slice(data))
    }
}

impl From<Vec<u8>> for Message {
    fn from(data: Vec<u8>) -> Self {
        Message::Binary(data.into())
    }
}

impl From<Message> for Vec<u8> {
    fn from(msg: Message) -> Self {
        msg.into_data().to_vec()
    }
}

use std::fmt;

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

/// The kind of error that occurred during streaming.
#[derive(Clone, Copy, Debug)]
pub enum StreamBodyKind {
    /// An error occurred while decoding a frame or format.
    CodecError,

    /// An error occurred while reading the stream.
    InputOutputError,

    /// The maximum object length was exceeded.
    MaxLenReachedError,
}

impl fmt::Display for StreamBodyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CodecError => f.write_str("Frame/codec error"),
            Self::InputOutputError => f.write_str("I/O error"),
            Self::MaxLenReachedError => f.write_str("Max object length reached"),
        }
    }
}

/// The error that may occur when attempting to stream an [`hpx::Response`].
#[derive(Debug)]
pub struct StreamBodyError {
    kind: StreamBodyKind,
    source: Option<BoxedError>,
    message: Option<String>,
}

impl fmt::Display for StreamBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(message) = &self.message {
            write!(f, ": {}", message)?;
        }
        if let Some(e) = &self.source {
            write!(f, ": {}", e)?;
        }
        Ok(())
    }
}

impl std::error::Error for StreamBodyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

impl StreamBodyError {
    /// Create a new instance of an error.
    pub fn new(kind: StreamBodyKind, source: Option<BoxedError>, message: Option<String>) -> Self {
        Self {
            kind,
            source,
            message,
        }
    }

    /// The kind of error that occurred during streaming.
    pub const fn kind(&self) -> StreamBodyKind {
        self.kind
    }

    /// The actual error that occurred.
    pub fn source_ref(&self) -> Option<&BoxedError> {
        self.source.as_ref()
    }

    /// The message associated with the error.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

impl From<std::io::Error> for StreamBodyError {
    fn from(err: std::io::Error) -> Self {
        Self::new(StreamBodyKind::InputOutputError, Some(Box::new(err)), None)
    }
}

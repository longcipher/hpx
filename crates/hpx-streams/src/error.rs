type BoxedError = Box<dyn std::error::Error + Send + Sync>;

/// The kind of error that occurred during streaming.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum StreamBodyKind {
    /// An error occurred while decoding a frame or format.
    #[error("Frame/codec error")]
    CodecError,

    /// An error occurred while reading the stream.
    #[error("I/O error")]
    InputOutputError,

    /// The maximum object length was exceeded.
    #[error("Max object length reached")]
    MaxLenReachedError,
}

/// The error that may occur when attempting to stream an [`hpx::Response`].
#[derive(Debug)]
pub struct StreamBodyError {
    kind: StreamBodyKind,
    source: Option<BoxedError>,
    message: Option<String>,
}

impl std::fmt::Display for StreamBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(message) = &self.message {
            write!(f, ": {message}")?;
        }
        if let Some(e) = &self.source {
            write!(f, ": {e}")?;
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

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn kind_display_codec() {
        assert_eq!(
            format!("{}", StreamBodyKind::CodecError),
            "Frame/codec error"
        );
    }

    #[test]
    fn kind_display_io() {
        assert_eq!(format!("{}", StreamBodyKind::InputOutputError), "I/O error");
    }

    #[test]
    fn kind_display_max_len() {
        assert_eq!(
            format!("{}", StreamBodyKind::MaxLenReachedError),
            "Max object length reached"
        );
    }

    #[test]
    fn error_with_message_only() {
        let err = StreamBodyError::new(StreamBodyKind::CodecError, None, Some("bad input".into()));
        assert_eq!(format!("{err}"), "Frame/codec error: bad input");
        assert!(err.source().is_none());
        assert_eq!(err.message(), Some("bad input"));
    }

    #[test]
    fn error_with_source_only() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broke");
        let err = StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(io_err)), None);
        assert_eq!(format!("{err}"), "Frame/codec error: pipe broke");
        assert!(err.source().is_some());
        assert!(err.message().is_none());
    }

    #[test]
    fn error_with_message_and_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let err = StreamBodyError::new(
            StreamBodyKind::InputOutputError,
            Some(Box::new(io_err)),
            Some("read failed".into()),
        );
        let display = format!("{err}");
        assert!(display.starts_with("I/O error: read failed: timeout"));
        assert!(err.source().is_some());
        assert_eq!(err.message(), Some("read failed"));
    }

    #[test]
    fn error_kind_only() {
        let err = StreamBodyError::new(StreamBodyKind::MaxLenReachedError, None, None);
        assert_eq!(format!("{err}"), "Max object length reached");
        assert!(err.source().is_none());
        assert!(err.message().is_none());
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let err: StreamBodyError = io_err.into();
        assert!(matches!(err.kind(), StreamBodyKind::InputOutputError));
        assert!(err.source().is_some());
    }

    #[test]
    fn error_is_std_error() {
        let err = StreamBodyError::new(StreamBodyKind::CodecError, None, None);
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn source_ref_returns_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broke");
        let err = StreamBodyError::new(StreamBodyKind::CodecError, Some(Box::new(io_err)), None);
        assert!(
            err.source_ref().is_some(),
            "source_ref must return the wrapped source"
        );
    }

    #[test]
    fn source_ref_is_none_without_source() {
        let err = StreamBodyError::new(StreamBodyKind::CodecError, None, None);
        assert!(err.source_ref().is_none());
    }

    #[test]
    fn kind_is_copy() {
        let k = StreamBodyKind::CodecError;
        let k2 = k;
        assert_eq!(format!("{k}"), format!("{k2}"));
    }
}

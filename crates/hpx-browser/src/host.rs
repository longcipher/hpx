//! Engine handle for browser control.

/// Handle to the underlying browser engine.
#[derive(Debug, Clone)]
pub struct EngineHandle {
    // ponytail: placeholder for real engine connection
}

impl EngineHandle {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for EngineHandle {
    fn default() -> Self {
        Self::new()
    }
}

//! Canvas 2D rendering (skia-safe), WebGL parameter stubs, AudioContext
//! fingerprint. Feature-gated on `canvas`.

pub mod audio;
pub mod canvas2d;
pub mod path;
pub mod periodic_wave;
pub mod text;
pub mod webgl;

pub use audio::{AudioFingerprint, AudioParams, WaveType};
pub use canvas2d::Canvas2D;
pub use path::Path2D;
pub use webgl::WebGLParams;

/// Canvas encoding error.
#[derive(Debug, thiserror::Error)]
pub enum CanvasError {
    #[error("encoding failed: {0}")]
    Encoding(String),
}

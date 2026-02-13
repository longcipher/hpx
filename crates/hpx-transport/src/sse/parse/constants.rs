//! Common constants used across the SSE parser.

use bytes_utils::Str;

/// Newline byte
pub(crate) const LF: u8 = b'\n';
/// Carriage return byte
pub(crate) const CR: u8 = b'\r';

/// Byte Order Mark as char
const BOM_CHAR: char = '\u{FEFF}';
const BOM_LEN: usize = BOM_CHAR.len_utf8();
// bom           = %xFEFF ; U+FEFF BYTE ORDER MARK
/// Byte representation of the BOM [`char`]
pub(crate) const BOM: &[u8; BOM_LEN] = &{
    let mut buf = [0u8; BOM_LEN];
    BOM_CHAR.encode_utf8(&mut buf);
    buf
};

/// Empty instance of [`Str`], from an `&'static ""`
pub(crate) const EMPTY_STR: Str = Str::from_static("");
/// Default event type string (`"message"`)
pub(crate) const MESSAGE_STR: Str = Str::from_static("message");

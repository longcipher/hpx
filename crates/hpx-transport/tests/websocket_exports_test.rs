use hpx_transport::websocket::{
    Connection, ConnectionEpoch, ConnectionHandle, ConnectionStream, Event,
};

#[test]
fn websocket_exports_compile() {
    let _ = std::mem::size_of::<ConnectionEpoch>();
    let _ = std::mem::size_of::<ConnectionHandle>();
    let _ = std::mem::size_of::<ConnectionStream>();
    let _ = std::mem::size_of::<Event>();
    let _ = std::mem::size_of::<Connection>();
}

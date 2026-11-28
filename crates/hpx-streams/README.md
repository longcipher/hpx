# hpx-streams

Streaming response support for [hpx](https://docs.rs/hpx/latest/hpx/) — decode
large response bodies frame by frame instead of buffering them whole.

Supported formats:

| Format | Feature | Extension trait |
| --- | --- | --- |
| JSON array | `json` | `JsonStreamResponse` |
| JSON Lines (NDJSON) | `json` | `JsonStreamResponse` |
| CSV | `csv` | `CsvStreamResponse` |
| Protobuf (length-prefixed) | `protobuf` | `ProtobufStreamResponse` |
| Apache Arrow IPC | `arrow` | `ArrowIpcStreamResponse` |
| FIX SBE (length-prefixed) | `sbe` | `SbeStreamResponse` |
| Borrowed JSON frames | `borrowed-json` | `BorrowedJsonFrame` |

The `default` feature set includes no format — pick the ones you need.

```rust,no_run
use hpx_streams::JsonStreamResponse as _;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct Order {
    symbol: String,
    price: f64,
}

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = hpx::Client::new();
let mut stream = client
    .get("http://localhost:8080/orders")
    .send()
    .await?
    .json_array_stream::<Order>(1024);

while let Some(order) = stream.next().await {
    println!("{:?}", order?);
}
# Ok(())
# }
```

## Zero-copy formats

The `sbe` and `borrowed-json` features decode without copying the payload:

- `sbe` frames a FIX SBE message behind a 4-byte big-endian length prefix and
  exposes the header plus a borrowed body. `SbeSchema` negotiates the schema
  id and version through the `Accept` and `X-MBX-SBE` request headers, and
  `SbeStreamResponse::sbe_refused` reports the documented `406` / `415`
  downgrade to JSON.
- `borrowed-json` returns a `BorrowedJsonFrame` that points into the `simd-json`
  tape, so a field is decoded only when it is actually read.

## License

Apache-2.0

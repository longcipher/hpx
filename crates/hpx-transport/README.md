# hpx-transport

Exchange transport governance for HFT clients — quota pools, dynamic auth and
order-book resync — with **no sockets, no TLS and no JSON**.

The crate is deliberately transport-agnostic. It is the deterministic policy
layer that sits between an exchange adapter and the `hpx` REST/WebSocket
clients, so every rule can be unit tested without a network.

| Module | Feature | What it does |
| --- | --- | --- |
| `rate_limit` | `rate-limit` | Three independent quota pools (IP request weight, account order rate, WebSocket frame rate) with server-response calibration, so local clock drift cannot cause `429` / `418` bans. |
| `auth` | `auth` | Per-message dynamic signing hooks for venues that require a fresh short-lived JWT for every subscription (Coinbase uses 120 s). |
| `book` | `book` | A snapshot + buffered-delta alignment state machine that replays deltas by sequence id after a reconnect. |

## Features

The `default` feature set enables all three modules. `full` is an alias for the
same set.

```toml
[dependencies]
hpx-transport = "2.6.1"
```

## Example

```rust,no_run
use std::time::Duration;

use hpx_transport::rate_limit::{QuotaConfig, QuotaKind, QuotaPool, SystemClock};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let pool = QuotaPool::new(
    QuotaKind::IpWeight,
    QuotaConfig::new(1200, Duration::from_secs(60)),
    SystemClock,
);

// A request that would exceed the quota is refused *before* it is sent.
pool.try_acquire(1)?;

// Calibrate against the value the server reports, so a drifting local clock
// cannot cause a ban.
pool.calibrate(1_150)?;
# Ok(())
# }
```

## Design notes

- Clocks are injectable through `rate_limit::Clock`, and `ManualClock` makes
  the quota windows deterministic in tests.
- All state is expressed with atomics, one short-lived mutex per quota window,
  or plain containers, so the crate stays free of hidden global state and
  background tasks.
- These are standalone crate features, not `hpx` feature flags, so they are
  not part of the `hft` or `stealth` presets.

## License

Apache-2.0

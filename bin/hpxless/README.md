# hpxless

Standalone CDP-compatible headless browser server built on [hpx-browser](../../crates/hpx-browser).

Start hpxless, point Puppeteer or Playwright at it — no Chrome required.

## Usage

```bash
# Start on default port 9222
hpxless

# Custom port with stealth enabled
hpxless --port 9222 --stealth

# Navigate to a URL on connect
hpxless --url https://example.com

# Block images and media
hpxless --block images,media

# Use a proxy
hpxless --proxy socks5://127.0.0.1:1080
```

## CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `9222` | WebSocket port for CDP server |
| `--stealth` | `false` | Enable anti-detection mode |
| `--profile` | `chrome` | Browser profile: `chrome`, `firefox`, `safari` |
| `--proxy` | — | Proxy URL (HTTP/HTTPS/SOCKS5) |
| `--block` | — | Comma-separated resource types to block |
| `--url` | — | Navigate to this URL on connect |
| `--log-level` | `info` | Log level: `trace`, `debug`, `info`, `warn`, `error` |

Environment variables: `HPXLESS_PORT`, `HPXLESS_PROXY`, `HPXLESS_LOG_LEVEL`.

## Puppeteer

```js
const puppeteer = require('puppeteer-core');

(async () => {
  const browser = await puppeteer.connect({
    browserWSEndpoint: 'ws://127.0.0.1:9222',
  });
  const page = await browser.newPage();
  await page.goto('https://example.com');
  console.log(await page.title());
  await browser.disconnect();
})();
```

## Playwright

```python
from playwright.sync_api import sync_playwright

with sync_playwright() as p:
    browser = p.chromium.connect_over_cdp('http://127.0.0.1:9222')
    context = browser.contexts[0]
    page = context.pages[0]
    page.goto('https://example.com')
    print(page.title())
    browser.close()
```

## Architecture

```text
hpxless
├── cli.rs          — clap CLI definition
├── main.rs         — entry point, tracing, CDP server startup
└── tests/          — BDD cucumber scenarios

hpx-browser (library)
├── page.rs         — Page navigation, resource loading pipeline
├── resource_loader.rs — Sub-resource discovery & concurrent fetch
├── js_runtime/     — V8 (deno_core) JavaScript runtime
├── protocol/       — CDP WebSocket server
├── dom.rs          — DOM abstraction (blitz-dom)
└── stealth.rs      — Anti-detection profiles
```

## License

Apache-2.0

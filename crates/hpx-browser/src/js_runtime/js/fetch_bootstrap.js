// Fetch bootstrap — wires globalThis.fetch to Rust op.
(() => {
  const core = Deno.core;

  globalThis.fetch = async (input, init = {}) => {
    const url = typeof input === "string" ? input : input.url || String(input);
    const method = (init.method || "GET").toUpperCase();

    const headers = [];
    if (init.headers) {
      const h = init.headers instanceof Headers ? init.headers : new Headers(init.headers);
      for (const [k, v] of h.entries()) headers.push([k, v]);
    }

    let body = "";
    if (init.body != null) {
      body = typeof init.body === "string" ? init.body : JSON.stringify(init.body);
    }

    // Async op: resolves to the JSON payload produced by `op_fetch`.
    const result = await core.ops.op_fetch(url, method, JSON.stringify(headers), body);

    let parsed;
    try {
      parsed = JSON.parse(result);
    } catch {
      parsed = { status: 0, url, headers: [], body: "", error: "parse error" };
    }

    return {
      status: parsed.status,
      statusText: parsed.statusText || "",
      url: parsed.url,
      headers: new Headers(parsed.headers || []),
      ok: parsed.status >= 200 && parsed.status < 300,
      text: async () => parsed.body || "",
      json: async () => JSON.parse(parsed.body || "null"),
      // ponytail: returns Uint8Array (not ArrayBuffer) and body is text — binary
    // responses (images, PDFs) are lost. Switch to an op that returns bytes and
    // use Uint8Array.buffer when binary fetch lands.
    arrayBuffer: async () => new TextEncoder().encode(parsed.body || ""),
    };
  };

  class Headers {
    constructor(init = []) {
      this._map = new Map();
      if (Array.isArray(init)) {
        for (const [k, v] of init) {
          this._map.set(k.toLowerCase(), v);
        }
      }
    }
    get(name) {
      return this._map.get(name.toLowerCase()) || null;
    }
    has(name) {
      return this._map.has(name.toLowerCase());
    }
    set(name, value) {
      this._map.set(name.toLowerCase(), value);
    }
    append(name, value) {
      this._map.set(name.toLowerCase(), value);
    }
    delete(name) {
      this._map.delete(name.toLowerCase());
    }
    *entries() {
      yield* this._map.entries();
    }
    *keys() {
      yield* this._map.keys();
    }
    *values() {
      yield* this._map.values();
    }
  }

  globalThis.Headers = Headers;
})();

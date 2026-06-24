// Fetch bootstrap — wires globalThis.fetch to Rust op.
(() => {
  const core = Deno.core;

  globalThis.fetch = async (input, init = {}) => {
    const url = typeof input === "string" ? input : input.url || String(input);
    const result = core.ops.op_fetch(url);

    let parsed;
    try {
      parsed = JSON.parse(result);
    } catch {
      parsed = { status: 0, url, headers: [], body: "", error: "parse error" };
    }

    return {
      status: parsed.status,
      statusText: parsed.status === 200 ? "OK" : "",
      url: parsed.url,
      headers: new Headers(parsed.headers || []),
      ok: parsed.status >= 200 && parsed.status < 300,
      text: async () => parsed.body || "",
      json: async () => JSON.parse(parsed.body || "null"),
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

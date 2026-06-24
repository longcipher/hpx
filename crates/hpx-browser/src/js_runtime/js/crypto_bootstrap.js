// Crypto bootstrap — wires globalThis.crypto to Rust ops.
(() => {
  const core = Deno.core;

  globalThis.crypto = {
    getRandomValues(arr) {
      if (!(arr instanceof TypedArray)) {
        throw new TypeError("Expected a TypedArray");
      }
      core.ops.op_crypto_random_fill(new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength));
      return arr;
    },
    async digest(algorithm, data) {
      const alg = typeof algorithm === "string" ? algorithm : algorithm.name;
      const input = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer || data);
      const result = core.ops.op_crypto_digest(alg, input);
      return result.buffer;
    },
    randomUUID() {
      // ponytail: simple UUID v4 using random bytes
      const buf = new Uint8Array(16);
      core.ops.op_crypto_random_fill(buf);
      buf[6] = (buf[6] & 0x0f) | 0x40;
      buf[8] = (buf[8] & 0x3f) | 0x80;
      const hex = Array.from(buf, (b) => b.toString(16).padStart(2, "0")).join("");
      return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    },
  };

  // TypedArray check helper
  const TypedArray = Object.getPrototypeOf(Uint8Array.prototype).constructor;
})();

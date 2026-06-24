// Storage bootstrap — wires localStorage/sessionStorage to Rust ops.
(() => {
  const core = Deno.core;

  class Storage {
    constructor(area) {
      this._area = area;
    }
    get length() {
      return core.ops.op_dom_storage_keys(this._area).length;
    }
    key(index) {
      const keys = core.ops.op_dom_storage_keys(this._area);
      return index >= 0 && index < keys.length ? keys[index] : null;
    }
    getItem(key) {
      const v = core.ops.op_dom_storage_get(this._area, key);
      return v === undefined ? null : v;
    }
    setItem(key, value) {
      core.ops.op_dom_storage_set(this._area, key, String(value));
    }
    removeItem(key) {
      core.ops.op_dom_storage_remove(this._area, key);
    }
    clear() {
      core.ops.op_dom_storage_clear(this._area);
    }
  }

  globalThis.localStorage = new Storage("local");
  globalThis.sessionStorage = new Storage("session");
})();

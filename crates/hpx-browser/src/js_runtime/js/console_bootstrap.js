// Console bootstrap — wires globalThis.console to Rust ops.
(() => {
  const core = Deno.core;
  const formatArgs = (...args) =>
    args
      .map((a) => {
        if (a === null) return "null";
        if (a === undefined) return "undefined";
        if (typeof a === "object") {
          try {
            return JSON.stringify(a);
          } catch {
            return String(a);
          }
        }
        return String(a);
      })
      .join(" ");

  globalThis.console = {
    log: (...args) => core.ops.op_console_log(formatArgs(...args)),
    warn: (...args) => core.ops.op_console_warn(formatArgs(...args)),
    error: (...args) => core.ops.op_console_error(formatArgs(...args)),
    info: (...args) => core.ops.op_console_log(formatArgs(...args)),
    debug: (...args) => core.ops.op_console_log(formatArgs(...args)),
  };
})();

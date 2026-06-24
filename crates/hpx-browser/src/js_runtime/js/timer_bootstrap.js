// Timer bootstrap — wires setTimeout/setInterval/clearTimeout/clearInterval to Rust ops.
(() => {
  const ops = Deno.core.ops;
  const pendingCallbacks = new Map();
  let nextCallbackId = 1;

  const fireCallback = (id) => {
    const cb = pendingCallbacks.get(id);
    if (!cb) return;
    if (cb.isInterval) {
      try {
        cb.fn();
      } catch (e) {
        console.error("Interval callback error:", e);
      }
      // Schedule next interval
      const run = async () => {
        await ops.op_timer_sleep(cb.delay);
        if (pendingCallbacks.has(id)) {
          try {
            cb.fn();
          } catch (e) {
            console.error("Interval callback error:", e);
          }
          run();
        }
      };
      run();
    } else {
      pendingCallbacks.delete(id);
      try {
        cb.fn();
      } catch (e) {
        console.error("Timer callback error:", e);
      }
    }
  };

  globalThis.setTimeout = (fn, delay = 0) => {
    const cbId = nextCallbackId++;
    const effectiveDelay = Math.max(delay, 0);
    const timerId = ops.op_set_timeout(effectiveDelay);
    pendingCallbacks.set(cbId, {
      fn,
      delay: effectiveDelay,
      isInterval: false,
      timerId,
    });

    (async () => {
      await ops.op_timer_sleep(effectiveDelay);
      fireCallback(cbId);
    })();

    return cbId;
  };

  globalThis.setInterval = (fn, delay = 0) => {
    const cbId = nextCallbackId++;
    const effectiveDelay = Math.max(delay, 4);
    const timerId = ops.op_set_interval(effectiveDelay);
    pendingCallbacks.set(cbId, {
      fn,
      delay: effectiveDelay,
      isInterval: true,
      timerId,
    });

    const run = async () => {
      await ops.op_timer_sleep(effectiveDelay);
      if (pendingCallbacks.has(cbId)) {
        try {
          fn();
        } catch (e) {
          console.error("Interval callback error:", e);
        }
        run();
      }
    };
    run();

    return cbId;
  };

  globalThis.clearTimeout = (id) => {
    const cb = pendingCallbacks.get(id);
    if (cb) {
      ops.op_clear_timer(cb.timerId);
      pendingCallbacks.delete(id);
    }
  };

  globalThis.clearInterval = (id) => {
    const cb = pendingCallbacks.get(id);
    if (cb) {
      ops.op_clear_timer(cb.timerId);
      pendingCallbacks.delete(id);
    }
  };
})();

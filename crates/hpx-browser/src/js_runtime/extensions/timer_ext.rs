use std::collections::HashMap;

use deno_core::{OpState, op2};

pub struct TimerState {
    next_id: i32,
    pub pending: HashMap<i32, TimerInfo>,
    pub cancelled: std::collections::HashSet<i32>,
}

#[derive(Debug, Clone)]
pub struct TimerInfo {
    pub delay_ms: u64,
    pub is_interval: bool,
}

impl Default for TimerState {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerState {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            pending: HashMap::new(),
            cancelled: std::collections::HashSet::new(),
        }
    }
}

#[op2(fast)]
#[smi]
pub(crate) fn op_set_timeout(state: &mut OpState, #[smi] delay_ms: i32) -> i32 {
    let state = state.borrow_mut::<TimerState>();
    let id = state.next_id;
    state.next_id += 1;
    state.pending.insert(
        id,
        TimerInfo {
            delay_ms: delay_ms.max(0) as u64,
            is_interval: false,
        },
    );
    id
}

#[op2(fast)]
#[smi]
pub(crate) fn op_set_interval(state: &mut OpState, #[smi] delay_ms: i32) -> i32 {
    let state = state.borrow_mut::<TimerState>();
    let id = state.next_id;
    state.next_id += 1;
    state.pending.insert(
        id,
        TimerInfo {
            delay_ms: delay_ms.max(4) as u64,
            is_interval: true,
        },
    );
    id
}

#[op2(fast)]
pub(crate) fn op_clear_timer(state: &mut OpState, #[smi] id: i32) {
    let state = state.borrow_mut::<TimerState>();
    state.cancelled.insert(id);
    state.pending.remove(&id);
}

#[op2(async(deferred), fast)]
pub(crate) async fn op_timer_sleep(#[smi] ms: i32) {
    tokio::time::sleep(tokio::time::Duration::from_millis(ms.max(0) as u64)).await;
}

deno_core::extension!(
    timer_extension,
    ops = [
        op_set_timeout,
        op_set_interval,
        op_clear_timer,
        op_timer_sleep
    ],
);

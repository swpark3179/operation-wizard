//! Call pacing for the FabriX LLM Serving API (D91).
//!
//! A FabriX licence allows a fixed number of API calls per minute (typically
//! 10). Every round of the tool loop (`fabrix.rs::drive`) is one
//! `POST /chat/completions` — the file tools themselves run locally and cost
//! nothing — so fast tool rounds used to exhaust the quota within a minute and
//! the turn died on the 429.
//!
//! One **process-wide** sliding-window gate paces every FabriX call (the main
//! conversation, prefill / classification / RAG-relevance side turns, the
//! knowledge summary — they all share one licence): a call waits until fewer
//! than `limit` calls were sent in the last minute. A sliding window that never
//! holds more than `limit` calls also keeps every fixed one-minute window under
//! the limit, whatever the gateway uses. A 429 that still arrives (another
//! client on the same key, a gateway counting differently) blocks the gate for
//! all runs until the computed wait has passed.
//!
//! The decision logic is pure (`next_slot`, `rate_limited_wait`) and unit-tested
//! with injected instants; only `acquire`/`on_rate_limited`/`record` touch the
//! global state.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::settings::FabrixConfig;

/// Calls per minute when the FabriX card sets none (the common licence).
pub const DEFAULT_CALLS_PER_MINUTE: u32 = 10;
/// The quota window plus a one-second margin (gateway clock skew, latency).
pub const WINDOW: Duration = Duration::from_secs(61);
/// Longest wait honoured after a 429; a longer `Retry-After` means a daily or
/// monthly quota, where waiting inside the turn makes no sense.
pub const MAX_RATE_WAIT: Duration = Duration::from_secs(90);
/// How many times one request is re-sent after a 429.
pub const MAX_RATE_RETRIES: u32 = 2;
/// Floor of a 429 wait estimated from the gate (no `Retry-After`).
const MIN_429_WAIT: Duration = Duration::from_secs(15);
/// 429 wait when nothing better is known.
const FALLBACK_429_WAIT: Duration = Duration::from_secs(60);

struct GateState {
    /// Send instants of the calls in the current window (oldest first).
    sent: VecDeque<Instant>,
    /// Set after a 429: no call may go out before this instant.
    blocked_until: Option<Instant>,
}

static GATE: Mutex<GateState> = Mutex::new(GateState { sent: VecDeque::new(), blocked_until: None });

/// The configured calls-per-minute limit (`None` → 10; `0` = unlimited).
pub fn limit_of(cfg: &FabrixConfig) -> u32 {
    cfg.calls_per_minute.unwrap_or(DEFAULT_CALLS_PER_MINUTE)
}

/// How long until one more call may be sent (`None` = now). Drops the stamps
/// that left the window. `limit == 0` disables pacing but a 429 block still
/// applies. Pure.
pub fn next_slot(
    sent: &mut VecDeque<Instant>,
    blocked_until: Option<Instant>,
    now: Instant,
    limit: u32,
    window: Duration,
) -> Option<Duration> {
    while sent.front().is_some_and(|t| now.saturating_duration_since(*t) >= window) {
        sent.pop_front();
    }
    let mut wait = blocked_until.map(|b| b.saturating_duration_since(now));
    let limit = limit as usize;
    if limit > 0 && sent.len() >= limit {
        // The call that must leave the window before one more fits.
        let free_at = sent[sent.len() - limit] + window;
        let w = free_at.saturating_duration_since(now);
        wait = Some(wait.map_or(w, |x| x.max(w)));
    }
    wait.filter(|d| !d.is_zero())
}

/// The wait before re-sending a request that got a 429 (`None` = give up).
/// `Retry-After` wins (too long → give up); otherwise the first retry waits
/// until the oldest call in the gate's window expires (≥ 15 s) and later ones a
/// full minute. Pure.
pub fn rate_limited_wait(
    retry_after: Option<u64>,
    sent: &VecDeque<Instant>,
    now: Instant,
    attempt: u32,
) -> Option<Duration> {
    if let Some(secs) = retry_after {
        let d = Duration::from_secs(secs.max(1));
        return (d <= MAX_RATE_WAIT).then_some(d);
    }
    let estimate = match sent.front() {
        Some(t) if attempt == 0 => (*t + WINDOW).saturating_duration_since(now),
        _ => FALLBACK_429_WAIT,
    };
    Some(estimate.max(MIN_429_WAIT).min(MAX_RATE_WAIT))
}

/// Whole seconds for a user-facing wait (rounded up, at least 1).
pub fn ceil_secs(d: Duration) -> u64 {
    let s = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    s.max(1)
}

/// Sleep `d`, polling the cancel flag; `true` if canceled meanwhile.
pub fn sleep_cancellable(d: Duration, cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + d;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return true;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        std::thread::sleep(left.min(Duration::from_millis(250)));
    }
}

fn gate() -> std::sync::MutexGuard<'static, GateState> {
    // A worker panic must not wedge every later FabriX call (D56 convention).
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Block until the quota allows one more call, then record it. `on_wait(secs)`
/// is called at the start of every wait (for the chat notice). `false` =
/// canceled while waiting.
pub fn acquire(limit: u32, cancel: &AtomicBool, on_wait: &dyn Fn(u64)) -> bool {
    loop {
        let wait = {
            let mut g = gate();
            let now = Instant::now();
            let blocked = g.blocked_until;
            match next_slot(&mut g.sent, blocked, now, limit, WINDOW) {
                None => {
                    g.sent.push_back(now);
                    return true;
                }
                Some(w) => w,
            }
        };
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        on_wait(ceil_secs(wait));
        if sleep_cancellable(wait, cancel) {
            return false;
        }
    }
}

/// Count a call that is not paced (the short model-list GET at detection) so
/// the gate's picture of the window stays accurate.
pub fn record() {
    gate().sent.push_back(Instant::now());
}

/// A 429 arrived: compute the wait before re-sending (`None` = give up) and
/// block the gate for every run until then.
pub fn on_rate_limited(retry_after: Option<u64>, attempt: u32) -> Option<Duration> {
    let mut g = gate();
    let now = Instant::now();
    let wait = rate_limited_wait(retry_after, &g.sent, now, attempt)?;
    let until = now + wait;
    g.blocked_until = Some(g.blocked_until.map_or(until, |b| b.max(until)));
    Some(wait)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamps(now: Instant, ago_secs: &[u64]) -> VecDeque<Instant> {
        ago_secs.iter().map(|s| now - Duration::from_secs(*s)).collect()
    }

    #[test]
    fn under_the_limit_passes_and_expired_stamps_drop() {
        let now = Instant::now() + Duration::from_secs(600);
        let mut sent = stamps(now, &[120, 90, 30, 10]);
        assert_eq!(next_slot(&mut sent, None, now, 10, WINDOW), None);
        assert_eq!(sent.len(), 2, "stamps older than the window are pruned");
    }

    #[test]
    fn at_the_limit_waits_for_the_oldest_to_expire() {
        let now = Instant::now() + Duration::from_secs(600);
        // 10 calls over the last 50 s; the oldest (50 s ago) frees at 61 s.
        let mut sent = stamps(now, &[50, 45, 40, 35, 30, 25, 20, 15, 10, 5]);
        assert_eq!(next_slot(&mut sent, None, now, 10, WINDOW), Some(Duration::from_secs(11)));
        // A lower limit waits for a younger stamp: limit 3 → the 3rd newest (15 s ago).
        assert_eq!(next_slot(&mut sent, None, now, 3, WINDOW), Some(Duration::from_secs(46)));
    }

    #[test]
    fn unlimited_still_honours_a_429_block() {
        let now = Instant::now() + Duration::from_secs(600);
        let mut sent = stamps(now, &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(next_slot(&mut sent, None, now, 0, WINDOW), None, "0 = no pacing");
        let block = Some(now + Duration::from_secs(7));
        assert_eq!(next_slot(&mut sent, block, now, 0, WINDOW), Some(Duration::from_secs(7)));
        // The longer of block and window wins.
        let mut full = stamps(now, &[50, 45, 40, 35, 30, 25, 20, 15, 10, 5]);
        assert_eq!(next_slot(&mut full, block, now, 10, WINDOW), Some(Duration::from_secs(11)));
        let past = Some(now - Duration::from_secs(3));
        assert_eq!(next_slot(&mut sent, past, now, 0, WINDOW), None, "an expired block is ignored");
    }

    #[test]
    fn rate_limited_wait_prefers_retry_after_and_gives_up_on_long_waits() {
        let now = Instant::now() + Duration::from_secs(600);
        let empty = VecDeque::new();
        assert_eq!(rate_limited_wait(Some(20), &empty, now, 0), Some(Duration::from_secs(20)));
        assert_eq!(rate_limited_wait(Some(0), &empty, now, 0), Some(Duration::from_secs(1)));
        assert_eq!(rate_limited_wait(Some(3600), &empty, now, 0), None, "a daily quota is not waited out");
        // No header: first retry waits for the oldest call in the window (≥ 15 s)…
        let sent = stamps(now, &[30, 20]);
        assert_eq!(rate_limited_wait(None, &sent, now, 0), Some(Duration::from_secs(31)));
        let recent = stamps(now, &[55]);
        assert_eq!(rate_limited_wait(None, &recent, now, 0), Some(Duration::from_secs(15)), "floored");
        // …later retries (or an empty gate) a full minute.
        assert_eq!(rate_limited_wait(None, &sent, now, 1), Some(Duration::from_secs(60)));
        assert_eq!(rate_limited_wait(None, &empty, now, 0), Some(Duration::from_secs(60)));
    }

    #[test]
    fn limit_defaults_to_ten_and_seconds_round_up() {
        let mut cfg = FabrixConfig::default();
        assert_eq!(limit_of(&cfg), 10);
        cfg.calls_per_minute = Some(0);
        assert_eq!(limit_of(&cfg), 0);
        assert_eq!(ceil_secs(Duration::from_millis(10_200)), 11);
        assert_eq!(ceil_secs(Duration::from_secs(5)), 5);
        assert_eq!(ceil_secs(Duration::ZERO), 1);
    }

    #[test]
    fn sleep_returns_early_on_cancel() {
        let cancel = AtomicBool::new(true);
        let t = Instant::now();
        assert!(sleep_cancellable(Duration::from_secs(30), &cancel));
        assert!(t.elapsed() < Duration::from_secs(1));
        assert!(!sleep_cancellable(Duration::from_millis(1), &AtomicBool::new(false)));
    }
}

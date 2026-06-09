//! Simple token-bucket-ish rate limiter for outbound HTTP calls.
//!
//! This is the same shape used in the binance + google_news
//! plugins (S34 / S35): a minimum interval between calls derived
//! from `rate_limit_per_sec`. Calls that would violate the
//! interval `await` the remainder on a tokio timer.
//!
//! The MVP uses the "1 req per `1/rate_limit_per_sec` seconds"
//! policy, which is conservative. A real bucket (e.g.
//! `governor`-style) would batch up to `rate_limit_per_sec`
//! requests without spacing; we don't need that granularity for
//! the S36 spec (InfluxDB v2 server-side throughput is far
//! higher than 100 req/s for the kinds of Pipelines the MVP
//! exercises).
//!
//! ## Threading
//!
//! The limiter is `Send + Sync` (internally uses `Arc<Mutex<Option<Instant>>>`).
//! The shared `last` field is the only mutable state; the
//! `Mutex<Option<Instant>>` is held only for the duration of the
//! comparison + update, never across an `.await`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RateLimiter {
    min_interval: Duration,
    last: Arc<Mutex<Option<Instant>>>,
}

impl RateLimiter {
    /// Build a limiter from the S36 spec default
    /// (`rate_limit_per_sec = 100` => 10 ms minimum interval).
    /// A value of `0` is treated as `1` to avoid division by
    /// zero.
    pub fn new(rate_limit_per_sec: u32) -> Self {
        let per_sec = rate_limit_per_sec.max(1) as f64;
        let min_interval = Duration::from_secs_f64(1.0 / per_sec);
        Self {
            min_interval,
            last: Arc::new(Mutex::new(None)),
        }
    }

    /// Wait until it's safe to make another call. Returns
    /// immediately on the first call; on subsequent calls waits
    /// until at least `min_interval` has elapsed since the last
    /// call.
    pub async fn wait(&self) {
        loop {
            let now = Instant::now();
            let should_wait = {
                let mut last = self.last.lock().expect("rate limiter poisoned");
                match *last {
                    Some(prev) if now.duration_since(prev) < self.min_interval => {
                        Some(self.min_interval - now.duration_since(prev))
                    }
                    _ => {
                        *last = Some(now);
                        None
                    }
                }
            };
            if let Some(d) = should_wait {
                tokio::time::sleep(d).await;
            } else {
                return;
            }
        }
    }
}

//! A Redis-backed [`RateLimitStore`] — the distributed rate-limit backend
//! (Seiva ADR 0015).
//!
//! The budget lives in Redis, so a client's limit holds across every replica in
//! a cluster (the in-process limiter enforces it only per instance). The check is
//! a single atomic Lua script — `INCR` the client's key, set its expiry on the
//! first hit of a window, read the remaining TTL — so concurrent replicas cannot
//! race past the limit. Redis is single-threaded and runs the script atomically.
//!
//! This crate is **optional** and not a dependency of the framework base; a
//! composition root wires it in only when it runs multiple replicas:
//!
//! ```no_run
//! # async fn f() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! use std::sync::Arc;
//! use std::time::Duration;
//! use seiva_ports::RateLimitStore;
//! use seiva_ratelimit_redis::RedisRateLimitStore;
//!
//! let store = Arc::new(RedisRateLimitStore::connect("redis://127.0.0.1:6379").await?);
//! // `store` satisfies `seiva_ports::RateLimitStore`; hand it to
//! // `seiva_http::DistributedRateLimit::new(store, limit)` and mount
//! // `seiva_http::rate_limit_with_store`. Used directly:
//! let _ = store.check("client-1.2.3.4", 300, Duration::from_secs(60)).await?;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

use std::time::Duration;

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use seiva_ports::{BoxError, PortResult, RateLimitDecision, RateLimitStore};

/// A fixed-window rate-limit store backed by Redis. Cloning shares the pooled,
/// multiplexed connection manager.
#[derive(Clone)]
pub struct RedisRateLimitStore {
    conn: ConnectionManager,
    prefix: String,
}

/// `INCR` the key; on the first hit of the window set its expiry; return the
/// current count and the remaining TTL (ms). One atomic round trip, so N
/// concurrent replicas see a single, correct counter.
const SCRIPT: &str = r"
local n = redis.call('INCR', KEYS[1])
if n == 1 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
local ttl = redis.call('PTTL', KEYS[1])
return {n, ttl}
";

impl RedisRateLimitStore {
    /// Connects to Redis at `url` (e.g. `redis://127.0.0.1:6379`) with the default
    /// key prefix `seiva:rl:`.
    ///
    /// # Errors
    ///
    /// Returns the Redis error if the URL is invalid or the initial connection
    /// cannot be established.
    pub async fn connect(url: &str) -> PortResult<Self> {
        let client = redis::Client::open(url).map_err(box_err)?;
        let conn = ConnectionManager::new(client).await.map_err(box_err)?;
        Ok(Self {
            conn,
            prefix: "seiva:rl:".to_owned(),
        })
    }

    /// Overrides the key prefix (default `seiva:rl:`).
    #[must_use]
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }
}

fn box_err(e: redis::RedisError) -> BoxError {
    Box::new(e) as BoxError
}

#[async_trait]
impl RateLimitStore for RedisRateLimitStore {
    async fn check(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> PortResult<RateLimitDecision> {
        let redis_key = format!("{}{key}", self.prefix);
        let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
        let mut conn = self.conn.clone();
        let (count, ttl_ms): (i64, i64) = redis::Script::new(SCRIPT)
            .key(redis_key)
            .arg(window_ms)
            .invoke_async(&mut conn)
            .await
            .map_err(box_err)?;

        if count <= i64::from(limit) {
            Ok(RateLimitDecision::Permitted)
        } else {
            // PTTL is -1 (no expiry) / -2 (missing) in edge cases; fall back to
            // the full window so a client is never told to retry "immediately".
            let retry = if ttl_ms > 0 {
                Duration::from_millis(ttl_ms as u64)
            } else {
                window
            };
            Ok(RateLimitDecision::Limited { retry_after: retry })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Live proof against a real Redis (ADR 0015): many concurrent clients share
    /// one budget, so exactly `limit` are permitted and the rest are limited —
    /// the guarantee a single-process limiter cannot give across replicas. Gated
    /// on `SEIVA_REDIS_TEST_URL`; skips cleanly when unset.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn live_distributed_budget_is_shared_and_atomic() {
        let Ok(url) = std::env::var("SEIVA_REDIS_TEST_URL") else {
            eprintln!("skipping: set SEIVA_REDIS_TEST_URL to run the live Redis proof");
            return;
        };
        let store = Arc::new(
            RedisRateLimitStore::connect(&url)
                .await
                .expect("connect to the test Redis")
                .with_prefix("seiva:rltest:"),
        );
        // A fresh key for this run.
        let key = "shared-client";
        {
            let mut conn = store.conn.clone();
            let _: () = redis::cmd("DEL")
                .arg(format!("seiva:rltest:{key}"))
                .query_async(&mut conn)
                .await
                .expect("clean the test key");
        }

        const N: usize = 50;
        const LIMIT: u32 = 10;
        let window = Duration::from_secs(60);

        let mut handles = Vec::new();
        for _ in 0..N {
            let store = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                store.check(key, LIMIT, window).await.expect("check")
            }));
        }
        let mut permitted = 0usize;
        for h in handles {
            if h.await.expect("task") == RateLimitDecision::Permitted {
                permitted += 1;
            }
        }
        assert_eq!(
            permitted, LIMIT as usize,
            "exactly the limit is permitted across {N} concurrent clients"
        );

        // A further check is limited, with a positive retry-after.
        match store.check(key, LIMIT, window).await.expect("check") {
            RateLimitDecision::Limited { retry_after } => {
                assert!(retry_after > Duration::ZERO, "retry-after must be positive");
            }
            RateLimitDecision::Permitted => panic!("over budget should be Limited"),
        }

        // Cleanup.
        let mut conn = store.conn.clone();
        let _: () = redis::cmd("DEL")
            .arg(format!("seiva:rltest:{key}"))
            .query_async(&mut conn)
            .await
            .expect("cleanup");
    }
}

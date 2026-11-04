# seiva-webhooks

Sign and verify webhooks per the [Standard Webhooks](https://www.standardwebhooks.com/)
specification, for the Delonix Seiva framework. Data-layer-free (DORM-free).

The signature is `HMAC-SHA256(secret, "{id}.{timestamp}.{payload}")`, base64-encoded
and emitted as `v1,<base64>`. Verification recomputes the MAC, compares it in
constant time, and rejects messages whose timestamp falls outside a replay window
(default ±5 minutes).

## Example

```rust
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use seiva_webhooks::{SignedHeaders, Webhook, DEFAULT_TOLERANCE};

// Base64 secret, optionally `whsec_`-prefixed; `from_bytes` takes a raw key.
let wh = Webhook::new("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap();

// Outgoing: sign, then send `webhook-id`, `webhook-timestamp`, `webhook-signature`.
let ts = 1_614_265_330_i64;
let payload = br#"{"event":"ping"}"#;
let sig = wh.sign("msg_abc", ts, payload);

// Incoming: verify against the headers and body, with an injected "now".
let headers = SignedHeaders { id: "msg_abc", timestamp: "1614265330", signature: sig.as_str() };
let now = UNIX_EPOCH + Duration::from_secs(ts as u64);
wh.verify(&headers, payload, now, DEFAULT_TOLERANCE).unwrap();
```

## License

MIT OR Apache-2.0.

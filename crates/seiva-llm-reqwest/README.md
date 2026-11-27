# seiva-llm-reqwest

The reqwest/rustls `HttpTransport` for [`seiva-llm`](../seiva-llm) (ADR 0048, G1).

- **Never follows a redirect**: a `3xx` comes back as a status, so the credential header
  never reaches another host. `ReqwestTransport::client_builder()` starts from
  `redirect::Policy::none()`; a client passed to `ReqwestTransport::new` must keep it.
- **Credential headers are marked sensitive** for reqwest and hyper.
- **No response at all** is `LlmError::Transport`, with the URL stripped from the message.

Excluded from the Seiva workspace (reqwest + rustls are heavy) and built by its own CI job,
which also runs an end-to-end test over real HTTP against a local server.

```rust
let transport = std::sync::Arc::new(seiva_llm_reqwest::ReqwestTransport::with_defaults()?);
```

## License

MIT OR Apache-2.0.

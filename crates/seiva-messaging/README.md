# seiva-messaging

Messaging for [Delonix Seiva](../../README.md): a **CloudEvents 1.0** envelope, a
`Publisher` trait, and broker backends. Moved out of DORM with its git history
(the ORM/framework boundary — DORM ADR 0054): broker clients are cross-service
communication, so they belong to the framework, not the ORM. **Data-layer-free** —
this crate carries bytes to a broker and never touches a database.

- `CloudEvent` — the CloudEvents 1.0 envelope (`id`, `source`, `type`,
  `specversion`, optional `time`/`traceparent`, JSON `data`).
- `Publisher` — publish a `CloudEvent` to a topic.
- Broker backends behind features: **NATS** (`nats`), **AMQP/RabbitMQ** (`amqp`),
  **Kafka** (`kafka`). A build that uses none pulls no client library.

The transactional-outbox → broker relay (the old `OutboxBus`) is **not** here:
consuming the in-database outbox is DORM's (`dorm-outbox`), and the relay loop
that drives it and publishes through a `Publisher` is a Seiva worker over that
crate.

//! The **typed event contract** (ADR 0027): a domain event is a published,
//! versioned, schema-bearing type — not an ad-hoc `json!` blob typed at the
//! `publish` call site.
//!
//! ADR 0021 fixed the event *transport* (transactional outbox, CloudEvents 1.0 on
//! the wire, at-least-once with inbox dedup, the same subscriber code in a monolith
//! and after a `module split`). It left the event *contract* empty: the producing
//! module hand-built an anonymous JSON object and the consumer hoped the bytes
//! matched the shape it deserialised into — the one coupling the framework calls
//! **primary** had no published language, while the synchronous facade was a typed,
//! versioned, `WireSchema`-published contract (ADR 0007). That asymmetry is gap
//! **A2** of the architecture review.
//!
//! This module closes the *contract* half without touching the transport:
//!
//! - [`DomainEvent`] — the event payload declares its **versioned topic** once, next
//!   to the type, and derives the same `serde` + [`WireSchema`]
//!   as a facade DTO, so a producer and the CloudEvent `type` cannot drift and the
//!   body is as discoverable as a response DTO.
//! - [`register_event!`](crate::register_event) + [`registered_events`] — a link-time
//!   catalogue, the exact mechanism facades (`registered_facades`), DTO schemas
//!   (`registered_schema`) and seeds (`registered_seeds`) already use, so a binary
//!   can emit the events it produces (a future `seiva events`) and a fitness test can
//!   assert the emitted bytes match the registered schema.
//! - [`publish_event`] — a thin, typed convenience over the **unchanged**
//!   [`EventPublisher`] byte port: it serialises the typed value and publishes it
//!   under the type's declared [`TOPIC`](DomainEvent::TOPIC). The application still
//!   publishes through the same port, inside the same `UnitOfWork::transaction`, into
//!   the same outbox (ADR 0021 holds); only the payload it hands over is a typed value
//!   under a declared topic instead of a `json!` literal under a free string.
//!
//! What stays out of this slice (tracked, not forgotten): the `#[derive(Event)]`
//! proc-macro and the scaffold emitting the typed shape, migrating the existing
//! `json!` events, and a `seiva events` subcommand. The contract, the catalogue and
//! the typed publish land first; they are what a `#[derive(Event)]` would target and
//! what `seiva events` would read.

use seiva_ports::{BoxError, EventPublisher, PortResult};
use serde_json::Value;

use crate::schema::WireSchema;

/// A **domain event payload**: a typed, versioned, schema-bearing contract for a
/// module's asynchronous boundary — the Published Language of the channel the
/// framework calls primary (ADR 0027).
///
/// A producing module declares the event body as a struct in its `facade` crate —
/// the same layer, and the same contract discipline, as its request/response DTOs
/// (ADR 0003) — deriving `serde::Serialize`/`Deserialize` and
/// [`WireSchema`], then implements this trait to name its
/// topic:
///
/// ```
/// use delonix_seiva::event::DomainEvent;
/// use delonix_seiva::WireSchema;
/// use serde::Serialize;
///
/// #[derive(Serialize, WireSchema)]
/// struct RecordCreated {
///     id: String,
///     title: String,
/// }
///
/// impl DomainEvent for RecordCreated {
///     const TOPIC: &'static str = "notes.record.created.v1";
/// }
/// ```
///
/// **Versioned topic.** [`TOPIC`](DomainEvent::TOPIC) is the CloudEvent `type`
/// string, declared **once** next to the payload with an explicit `.vN` suffix. The
/// version is part of the contract: a breaking payload change is a new `…​.v2`
/// published alongside `…​.v1` until consumers migrate — the asynchronous twin of
/// facade versioning. [`EventContract::version`] reads it back, and
/// [`unversioned_event_topics`] is the lint that rejects a topic carrying no version.
///
/// It names no transport: whether the publish is transactional is a property of the
/// [`EventPublisher`] the application holds, exactly as before.
pub trait DomainEvent: WireSchema + serde::Serialize {
    /// The declared, versioned CloudEvent `type` for this event, e.g.
    /// `"notes.record.created.v1"`. The single source of the topic string, so the
    /// producer and the CloudEvent `ty` cannot drift.
    const TOPIC: &'static str;
}

/// One registered event in the link-time catalogue: its declared topic, the Rust
/// type name that produced it, and a function yielding the payload's
/// [`WireSchema`] fragment.
///
/// All fields are `&'static` / a function pointer, so a registration is a `const`
/// expression [`inventory::submit!`](crate::inventory) can take at item scope — the
/// same shape as a [`SchemaRegistration`](crate::schema::SchemaRegistration).
#[derive(Clone, Copy)]
pub struct EventContract {
    /// The declared, versioned topic — the CloudEvent `type` (`"orders.placed.v1"`).
    pub topic: &'static str,
    /// The payload's Rust type name, as written at the registration site
    /// (`"RecordCreated"`) — the counterpart of a facade descriptor's type name.
    pub type_name: &'static str,
    /// The payload type's schema contribution, identical to what `WireSchema` emits
    /// for a facade DTO — so an event is as discoverable as a route.
    pub schema: fn() -> Value,
}

impl EventContract {
    /// The payload's JSON Schema fragment (calls the registered [`schema`](Self::schema)).
    #[must_use]
    pub fn schema(&self) -> Value {
        (self.schema)()
    }

    /// The version segment of the [`topic`](Self::topic) — the trailing `.vN` — or
    /// `None` when the topic carries no version suffix.
    ///
    /// A version is the last dot-separated segment shaped `v` followed by one or more
    /// digits (`"orders.placed.v1"` → `Some("v1")`); anything else is unversioned
    /// (`"orders.placed"` → `None`), which [`unversioned_event_topics`] flags.
    #[must_use]
    pub fn version(&self) -> Option<&'static str> {
        let last = self.topic.rsplit('.').next()?;
        let digits = last.strip_prefix('v')?;
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            Some(last)
        } else {
            None
        }
    }
}

/// An event contract's registration in the link-time inventory.
///
/// [`register_event!`](crate::register_event) emits one
/// [`inventory::submit!`](crate::inventory) of this per event type, so every event
/// *linked into a binary* is discoverable through [`registered_events`] — the same
/// mechanism facades, DTO schemas and seeds use.
pub struct EventRegistration {
    /// The registered event contract.
    pub contract: EventContract,
}

inventory::collect!(EventRegistration);

/// Every event registered via [`register_event!`](crate::register_event) and linked
/// into the current binary, sorted by `topic` for a deterministic order regardless
/// of link order.
///
/// A standalone tool (the `seiva` CLI) sees only the events compiled into it; the
/// composition-root binary that mounts the modules sees them all — so its catalogue
/// is the complete one, the asynchronous twin of the `/openapi.json` route list.
#[must_use]
pub fn registered_events() -> Vec<EventContract> {
    let mut events: Vec<EventContract> = inventory::iter::<EventRegistration>
        .into_iter()
        .map(|registration| registration.contract)
        .collect();
    events.sort_by_key(|contract| contract.topic);
    events
}

/// The topics registered more than once — a conflict so two events declaring the
/// same `TOPIC` fail loudly rather than silently sharing a wire type. Returns the
/// duplicated topics (each once), sorted; empty when consistent.
#[must_use]
pub fn event_registry_conflicts() -> Vec<&'static str> {
    let events = registered_events();
    let mut conflicts = Vec::new();
    // `events` is sorted by topic, so duplicates are adjacent.
    let mut i = 0;
    while i < events.len() {
        let topic = events[i].topic;
        let mut j = i + 1;
        while j < events.len() && events[j].topic == topic {
            j += 1;
        }
        if j - i > 1 {
            conflicts.push(topic);
        }
        i = j;
    }
    conflicts
}

/// The registered topics that carry no `.vN` version suffix — the fitness lint ADR
/// 0027 asks for ("the framework should lint a topic string that carries no
/// version"). Returns them sorted; empty when every event is versioned.
#[must_use]
pub fn unversioned_event_topics() -> Vec<&'static str> {
    registered_events()
        .into_iter()
        .filter(|contract| contract.version().is_none())
        .map(|contract| contract.topic)
        .collect()
}

/// Registers a [`DomainEvent`] type in the link-time event catalogue. A module
/// declares it from its `facade` crate (where the event type lives), next to the
/// payload: it expands to an [`inventory::submit!`](crate::inventory), so the call
/// must sit at item position (module scope), exactly like
/// [`register_seed!`](crate::register_seed).
///
/// ```
/// use delonix_seiva::event::DomainEvent;
/// use delonix_seiva::WireSchema;
/// use serde::Serialize;
///
/// #[derive(Serialize, WireSchema)]
/// struct RecordCreated { id: String }
/// impl DomainEvent for RecordCreated {
///     const TOPIC: &'static str = "notes.record.created.v1";
/// }
/// delonix_seiva::register_event!(RecordCreated);
/// ```
#[macro_export]
macro_rules! register_event {
    ($ev:ty $(,)?) => {
        $crate::inventory::submit! {
            $crate::event::EventRegistration {
                contract: $crate::event::EventContract {
                    topic: <$ev as $crate::event::DomainEvent>::TOPIC,
                    type_name: ::core::stringify!($ev),
                    schema: <$ev as $crate::schema::WireSchema>::schema,
                },
            }
        }
    };
}

/// Serialise a [`DomainEvent`] to the payload bytes a publish carries — the typed
/// value becoming the CloudEvent `data` body. A serialisation failure (a payload
/// that does not serialise) surfaces as a [`PortResult`] error rather than a panic.
///
/// This is the one place the typed shape becomes bytes; [`publish_event`] uses it,
/// and a fitness test asserts the bytes match the registered schema.
pub fn encode<E: DomainEvent>(event: &E) -> PortResult<Vec<u8>> {
    serde_json::to_vec(event).map_err(|e| Box::new(e) as BoxError)
}

/// Publish a typed [`DomainEvent`] through an [`EventPublisher`], under the event's
/// own declared [`TOPIC`](DomainEvent::TOPIC).
///
/// This is the typed convenience over the **unchanged** byte port (ADR 0027 keeps
/// the transport of ADR 0021): the application still publishes through the
/// `EventPublisher` it holds — inside its `UnitOfWork::transaction`, into the
/// transactional outbox, at-least-once with inbox dedup — but hands over a typed
/// value under a declared topic instead of a hand-built `json!` literal under a free
/// string. The subscriber's `from_slice::<E>` now has a producer-side guarantee
/// behind it.
///
/// A free function rather than a trait method on purpose: it needs no change to the
/// object-safe [`EventPublisher`] port and composes with any implementation (the
/// in-memory double, the DORM outbox publisher) unchanged.
pub async fn publish_event<P, E>(publisher: &P, event: &E) -> PortResult<()>
where
    P: EventPublisher + ?Sized,
    E: DomainEvent + Sync,
{
    let payload = encode(event)?;
    publisher.publish(E::TOPIC, &payload).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_testing::RecordingEventPublisher;
    use serde::Serialize;

    // A sample module's event, declared exactly as a producing module would in its
    // `facade` crate: a typed payload deriving `Serialize` + `WireSchema`, with its
    // versioned topic named once. `register_event!` sits at item scope (it expands to
    // an `inventory::submit!`), so the catalogue sees it when this test binary links.
    #[derive(Serialize, crate::WireSchema)]
    struct RecordCreated {
        id: String,
        title: String,
    }

    impl DomainEvent for RecordCreated {
        const TOPIC: &'static str = "notes.record.created.v1";
    }

    crate::register_event!(RecordCreated);

    #[test]
    fn the_catalogue_lists_the_registered_event_with_its_topic_and_schema() {
        let events = registered_events();
        let contract = events
            .iter()
            .find(|c| c.type_name == "RecordCreated")
            .expect("the registered event is in the link-time catalogue");

        // The declared, versioned topic is what the catalogue carries — the
        // asynchronous twin of a route in `/openapi.json` (ADR 0027 A2).
        assert_eq!(contract.topic, "notes.record.created.v1");

        // The payload schema is the SAME `WireSchema` a facade DTO publishes, so the
        // event body is as discoverable as a response DTO.
        let schema = contract.schema();
        assert_eq!(schema["properties"]["title"]["type"], "string");
        assert_eq!(schema["properties"]["id"]["type"], "string");
    }

    #[test]
    fn the_topic_is_versioned_and_the_lint_is_clean() {
        // The registered event carries a `.vN`, so the version lint reports nothing.
        let unversioned = unversioned_event_topics();
        assert!(
            !unversioned.contains(&"notes.record.created.v1"),
            "a versioned topic must not be flagged: {unversioned:?}"
        );

        let contract = EventContract {
            topic: RecordCreated::TOPIC,
            type_name: "RecordCreated",
            schema: <RecordCreated as WireSchema>::schema,
        };
        assert_eq!(contract.version(), Some("v1"));

        // A topic without a `.vN` suffix is what the lint catches.
        let bare = EventContract {
            topic: "notes.record.created",
            type_name: "RecordCreated",
            schema: <RecordCreated as WireSchema>::schema,
        };
        assert_eq!(bare.version(), None);
    }

    #[test]
    fn no_topic_is_registered_twice() {
        assert!(
            event_registry_conflicts().is_empty(),
            "a topic registered twice is a wire-contract collision"
        );
    }

    #[tokio::test]
    async fn typed_publish_uses_the_declared_versioned_topic_and_typed_body() {
        let publisher = RecordingEventPublisher::new();

        publish_event(
            &publisher,
            &RecordCreated {
                id: "7".to_owned(),
                title: "First note".to_owned(),
            },
        )
        .await
        .expect("the typed publish succeeds");

        let published = publisher.events();
        assert_eq!(published.len(), 1);
        // The publish went out under the event's OWN declared topic — a producer can
        // no longer drift the wire type from the payload.
        assert_eq!(published[0].topic, "notes.record.created.v1");
        // The bytes are the typed payload, deserialisable back into the shape the
        // registered schema describes (producer-side guarantee for the subscriber).
        let body: Value = serde_json::from_slice(&published[0].payload).unwrap();
        assert_eq!(body["id"], "7");
        assert_eq!(body["title"], "First note");
    }
}

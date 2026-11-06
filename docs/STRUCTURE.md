# Documentation structure — the Diátaxis layout

Seiva's documentation is organised by the four [Diátaxis](https://diataxis.fr)
modes — **tutorials · topic guides · reference · how-to** — the model that keeps
framework docs navigable: learning material, understanding material, information
material and task material each live in their own place. The goal: any backend
developer should find their way around Seiva's docs on instinct.

This page maps the top-level sections and the thematic layers, and notes where
Seiva's shape is distinctive (Seiva is API-first and compiled, so a few concerns —
HTML templates, a GUI admin, i18n — map to different things or are out of scope
for now).

## Top-level sections

| Section | Path | Notes |
| --- | --- | --- |
| Getting started | [`getting-started/`](getting-started/index.md) | at-a-glance, install, the tutorial |
| Topic guides ("Using Seiva") | [`topics/`](topics/index.md) | one understanding-oriented guide per layer |
| How-to guides | [`howto/`](howto/index.md) | task recipes |
| API reference | [`reference/`](reference/index.md) | CLI, settings, descriptor, crates |
| FAQ | [`faq/`](faq/index.md) | |
| Glossary | [`glossary.md`](glossary.md) | |
| Release notes | [`releases/`](releases/index.md) | + [engineering log](progress.md) |
| Internals | [`internals/`](internals/index.md) | architecture, ADRs, contributing |

## The thematic layers (topic guides)

| Concern | Seiva topic | Status |
| --- | --- | --- |
| The data layer | [The data layer (DORM)](topics/data-layer/index.md) | DORM is a separate product, wired via the `dorm` feature |
| The transport layer | [The transport layer](topics/transport/index.md) | facades + controllers, generated — not hand-written views |
| API documents | [API documents](topics/api-documents.md) | **distinctive:** Seiva is API-first → OpenAPI/docs UI/proto instead of HTML templates |
| Validation | [Validation and DTOs](topics/validation.md) | `Valid<T>` + `Validate`, RFC 9457 errors |
| The development process | [Testing](topics/testing.md), [Configuration](topics/configuration.md), [Packaging](topics/performance.md) | |
| Admin | — | **out of scope (for now):** an auto-CRUD admin is a roadmap item; see the competitive analysis |
| Security | [Security](topics/security.md), [Authorization](topics/authorization.md) | deny-by-default, BOLA by construction |
| Internationalization | — | out of scope for now |
| Performance & optimization | [Performance & cloud-native](topics/performance.md) | single binary, distroless |
| Common web app tools | [Authentication](topics/authentication.md), [Caching](topics/caching.md), [Observability](topics/observability.md), [Messaging](topics/messaging-and-events.md), [Pagination](topics/pagination.md), [Email](topics/sending-email.md), [Sessions](topics/sessions.md), [Serialization](topics/serialization.md) | |
| Other core functionalities | [Signals/events](topics/messaging-and-events.md), [CQRS](topics/cqrs.md), [Background workers](topics/background-workers.md) | |
| The open-source project | [Internals](internals/index.md), [ADRs](internals/adrs.md), [Contributing](internals/contributing.md), [Release process](internals/release-process.md) | |

## How to navigate

Reach for **getting-started** to learn by doing, **topics/** to understand a layer
in depth, **reference/** to look up an exact flag or setting, and **howto/** for a
focused recipe. The [documentation home](index.md) is the entry point.

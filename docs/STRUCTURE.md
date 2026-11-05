# Documentation structure — modelled on Django

Seiva's documentation deliberately mirrors the organisation of the
[Django 6.1 documentation](https://docs.djangoproject.com/en/6.1/), which is the
gold standard for framework docs (it is where the [Diátaxis](https://diataxis.fr)
model originated). The goal: a FastAPI/Django developer should find their way
around Seiva's docs on instinct.

This page maps every Django top-level section to its Seiva counterpart, and notes
where Seiva deliberately **differs** (Seiva is API-first and compiled, so a few
Django sections — templates, the admin GUI, i18n — map to different things or are
out of scope for now).

## Top-level (the Django `/contents/` tree)

| Django | Seiva | Notes |
| --- | --- | --- |
| Getting started | [`getting-started/`](getting-started/index.md) | at-a-glance, install, the tutorial |
| Using Django (topic guides) | [`topics/`](topics/index.md) | one guide per layer |
| How-to guides | [`howto/`](howto/index.md) | task recipes |
| API Reference | [`reference/`](reference/index.md) | CLI, settings, descriptor, crates |
| FAQ | [`faq/`](faq/index.md) | |
| Glossary | [`glossary.md`](glossary.md) | |
| Release notes | [`releases/`](releases/index.md) | + [engineering log](progress.md) |
| Django internals | [`internals/`](internals/index.md) | architecture, ADRs, contributing |

## The thematic layers (the Django landing page)

| Django section | Seiva topic | Status |
| --- | --- | --- |
| The model layer | [The data layer (DORM)](topics/data-layer/index.md) | DORM is a separate product, wired via the `dorm` feature |
| The view layer | [The transport layer](topics/transport/index.md) | facades + controllers, not view functions |
| The template layer | [API documents](topics/api-documents.md) | **differs:** Seiva is API-first → OpenAPI/docs UI/proto instead of HTML templates |
| Forms | [Validation and DTOs](topics/validation.md) | `Valid<T>` + `Validate`, RFC 9457 errors |
| The development process | [Testing](topics/testing.md), [Configuration](topics/configuration.md), [Packaging](topics/performance.md) | |
| The admin | — | **out of scope (for now):** an auto-CRUD admin is a roadmap item; see the competitive analysis |
| Security | [Security](topics/security.md), [Authorization](topics/authorization.md) | deny-by-default, BOLA by construction |
| Internationalization | — | out of scope for now |
| Performance & optimization | [Performance & cloud-native](topics/performance.md) | single binary, distroless |
| Geographic framework | — | out of scope |
| Common web app tools | [Authentication](topics/authentication.md), [Caching](topics/caching.md), [Observability](topics/observability.md), [Messaging](topics/messaging-and-events.md), [Pagination](topics/pagination.md), [Email](topics/sending-email.md), [Sessions](topics/sessions.md), [Serialization](topics/serialization.md) | |
| Other core functionalities | [Signals/events](topics/messaging-and-events.md), [CQRS](topics/cqrs.md), [Background workers](topics/background-workers.md) | |
| The open-source project | [Internals](internals/index.md), [ADRs](internals/adrs.md), [Contributing](internals/contributing.md), [Release process](internals/release-process.md) | |

## How this tree was built

The Django structure was captured by navigating
`https://docs.djangoproject.com/en/6.1/` with Playwright and extracting both the
landing-page sections (16) and the exhaustive `/contents/` table of contents
(≈700 nodes). The Seiva tree above is the faithful re-mapping; the
[documentation home](index.md) is the entry point.

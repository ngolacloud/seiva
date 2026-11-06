# Part 8: A browser frontend with dxjs

By Part 7 the poll-site is a complete backend: a contract-first Seiva service that
serves `GET /v1/polls`, `GET /v1/polls/{id}`, `POST /v1/polls/vote` and the
results, with OpenAPI and permissions derived from the facade. This part puts a
**browser UI** on it with [**dxjs**](https://github.com/ngolacloud/dxjs) — the
Delonix fine-grained reactive UI engine — so the poll-site becomes full-stack:
questions, choices, voting and live results in the browser, talking to the Seiva
API you already built.

!!! note "dxjs is the frontend engine; `dx` is the package manager"
    Don't confuse the two Delonix tools. [`dx`](../../dx/index.md) builds and runs
    your Rust project. **dxjs** is a TypeScript UI library (signals, no virtual
    DOM) that renders in the browser. They are unrelated beyond the family name.

dxjs uses **signals** for surgical DOM updates (a full reactive app is ~2.8 KB
gzip), and renders with no `eval` and no inline handlers, so it runs under a
strict Content-Security-Policy — the same posture the Seiva console admin needs.

## The packages

A dxjs app pulls three small packages, plus the transport client:

```ts
import { signal } from "@dxjs/core";          // reactivity
import { html } from "@dxjs/dom";             // escaped templates
import { component, mount } from "@dxjs/component";
import { createClient, resource } from "@dxjs/transport"; // REST to the Seiva API
```

dxjs runs TypeScript directly on Node ≥ 22 (native type-stripping), so there is no
build step to see it working.

## A client to the Seiva API

`createClient` wraps `fetch` for one origin. Point it at the running Seiva service
(same-origin in production, or the dev address):

```ts
const client = createClient({ baseUrl: "" }); // same origin as the page
```

Mirror your facade's DTOs as TypeScript types — the same shapes Part 2 published
to `/openapi.json`, so the wire contract is the single source of truth:

```ts
interface QuestionDto { id: string; text: string }
interface ChoiceDto   { id: string; question_id: string; text: string; votes: number }
interface QuestionDetail { question: QuestionDto; choices: ChoiceDto[] }
```

## The index view — latest questions

`resource` turns an async fetch into three signals — the value, `loading()` and
`error()` — plus `refetch()`. It is the reactive equivalent of the `list` use case
from Part 1:

```ts
const Questions = component(() => {
  const client = createClient({ baseUrl: "" });
  const questions = resource<QuestionDto[]>((sig) =>
    client.get<QuestionDto[]>("/v1/polls", { signal: sig }),
  );

  return html`
    <section>
      <h1>Polls</h1>
      ${() => questions.loading() ? html`<p>Loading…</p>` : null}
      ${() => questions.error() ? html`<p class="err">Could not load polls.</p>` : null}
      <ul>
        ${() => (questions() ?? []).map(
          (q) => html`<li><a href=${"#/q/" + q.id}>${q.text}</a></li>`,
        )}
      </ul>
    </section>`;
});
```

Every `${() => …}` binding is a surgical update: when a signal it reads changes,
only that node is patched — no re-render, no virtual DOM. Interpolated values
(`q.text`) go through escaped sinks, so a question titled `<script>…` is shown as
text, never executed.

## Detail, voting and results

`GET /v1/polls/{id}` returns the question with its choices; `POST /v1/polls/vote`
bumps a tally (it needs the `polls.vote` permission — Part 4). After a successful
vote we `refetch()` so the tally updates in place:

```ts
const QuestionView = component((props: { id: string; token: () => string }) => {
  const client = createClient({ baseUrl: "" });
  const auth = () => ({ authorization: "Bearer " + props.token() });

  const detail = resource<QuestionDetail>((sig) =>
    client.get<QuestionDetail>("/v1/polls/" + props.id, { signal: sig, headers: auth() }),
  );
  const status = signal("");

  const vote = (choiceId: string) => {
    status.set("…");
    client
      .post("/v1/polls/vote", { choice_id: choiceId }, { headers: auth() })
      .then(() => { status.set("voted"); detail.refetch(); })
      .catch((e: unknown) => status.set("vote failed: " + String(e)));
  };

  return html`
    <section>
      ${() => {
        const d = detail();
        if (!d) return html`<p>Loading…</p>`;
        return html`
          <h2>${d.question.text}</h2>
          <ul>
            ${d.choices.map((c) => html`
              <li>
                <button @click=${() => vote(c.id)}>Vote</button>
                ${c.text} — <strong>${c.votes}</strong>
              </li>`)}
          </ul>`;
      }}
      <p class="status">${() => status()}</p>
    </section>`;
});
```

The bearer token rides only in the `Authorization` header, never in a URL — the
same rule the backend expects. The `can`-style hiding of the vote button for a
read-only caller is presentational; the Seiva controller still enforces
`polls.vote` server-side, so the UI cannot grant access the contract denies.

## Mount it

```ts
mount(Questions, document.querySelector("#app"), {});
```

`mount` returns a disposer that tears down every signal subscription and DOM
binding — there is no leak when a view is replaced. For multi-view navigation
(`#/q/{id}` → the detail view), `@dxjs/component` ships `createRouter`.

## Live results (optional)

If the service streams its `polls.voted` events over Server-Sent Events,
`@dxjs/transport`'s `createLiveSource` subscribes to them and lets the results
update without polling — the tally rises as other people vote. The REST path
above works with no streaming; wire the live source only once the backend
publishes an event endpoint.

## The poll-site is full-stack

You now have the whole loop: a compiled Seiva backend whose contract drives the
routes, the OpenAPI and the permissions (Parts 1–6), the same module promotable to
its own service (Part 7), and a tiny, CSP-safe reactive UI (this part) that
consumes exactly that contract. The frontend holds no business rules — it reads
and writes through the API, and the server stays the single authority.

Where next:

- The [topic guides](../../topics/index.md) for depth on each backend layer.
- The [DORM section](../../dorm/index.md) to take the data layer further.
- The [dxjs design](https://github.com/ngolacloud/dxjs/blob/main/DESIGN.md) for the
  reactivity model, SSR/hydration and the transport in full.

---

*Part of the [Seiva documentation](../../index.md).*

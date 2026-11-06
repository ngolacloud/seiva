# Part 8: A browser frontend with dxjs

The shop is a complete backend — a contract-first service (or fleet of services,
after Part 7) with OpenAPI and permissions derived from each module's facade. This
part puts a **storefront** on it with [**dxjs**](https://github.com/ngolacloud/dxjs),
the Delonix fine-grained reactive UI engine: browse the catalogue and place an
order, talking to the shop API you built.

!!! note "dxjs is the frontend engine; `dx` is the package manager"
    Don't confuse the two Delonix tools. [`dx`](../../dx/index.md) builds and runs
    your Rust project. **dxjs** is a TypeScript UI library (signals, no virtual
    DOM) that renders in the browser.

dxjs uses **signals** for surgical DOM updates (a full reactive app is ~2.8 KB
gzip) and renders with no `eval` and no inline handlers, so it runs under a strict
Content-Security-Policy.

## The packages and a client

```ts
import { signal } from "@dxjs/core";
import { html } from "@dxjs/dom";
import { component, mount } from "@dxjs/component";
import { createClient, resource } from "@dxjs/transport";

const client = createClient({ baseUrl: "" }); // same origin as the shop
```

Mirror the shop's DTOs — the shapes Part 2 published to `/openapi.json`:

```ts
interface ProductDto { id: string; name: string; price_cents: number }
interface LineDto    { product_id: string; qty: number }
interface OrderDto   { id: string; customer: string; lines: LineDto[]; status: string }
```

## Browse the catalogue

`resource` turns an async fetch into reactive signals — the value, `loading()` and
`error()`. This is the `catalog.list` use case in the browser:

```ts
const Catalogue = component(() => {
  const products = resource<ProductDto[]>((sig) =>
    client.get<ProductDto[]>("/v1/catalog", { signal: sig }),
  );
  return html`
    <section>
      <h1>Shop</h1>
      ${() => products.loading() ? html`<p>Loading…</p>` : null}
      <ul>
        ${() => (products() ?? []).map((p) => html`
          <li>${p.name} — ${() => (p.price_cents / 100).toFixed(2)}
              <button @click=${() => addToCart(p.id)}>Add</button></li>`)}
      </ul>
    </section>`;
});
```

Every `${() => …}` binding is a surgical update, and interpolated values go through
escaped sinks — a product named `<script>…` is shown as text, never executed.

## Place an order

A small cart signal collects lines; placing the order is one `POST` to
`orders.place` (it needs the `orders.manage` permission — Part 5). The token rides
only in the `Authorization` header:

```ts
const cart = signal<LineDto[]>([]);
const addToCart = (productId: string) =>
  cart.update((lines) => [...lines, { product_id: productId, qty: 1 }]);

const placeOrder = (customer: string, token: string) =>
  client.post<OrderDto>(
    "/v1/orders",
    { customer, lines: cart() },
    { headers: { authorization: "Bearer " + token } },
  ).then((order) => {
    // The backend now runs the whole flow from Part 3: the order emits
    // `orders.placed`, which drives the reservation, the charge and the
    // confirmation — none of which the frontend has to orchestrate.
    cart.set([]);
    return order;
  });
```

The frontend holds no business rules: it reads the catalogue and places an order
through the contract, and the server runs the order flow and stays the single
authority. Whether `payments` is in the monolith or split out (Part 7) makes no
difference to this code.

## Mount it

```ts
mount(Catalogue, document.querySelector("#app"), {});
```

`mount` returns a disposer that tears down every subscription. For multi-view
navigation (catalogue → order confirmation), `@dxjs/component` ships
`createRouter`; for a live order status that updates as the backend emits events,
`@dxjs/transport`'s `createLiveSource` subscribes to a Server-Sent-Events stream.

## You built a shop — front to back

A five-module backend that runs as a monolith or as microservices, and a tiny,
CSP-safe reactive storefront that consumes exactly its contract. Where next:

- The [topic guides](../../topics/index.md) for depth on each layer.
- The [DORM section](../../dorm/index.md) and the [dx section](../../dx/index.md).
- The [dxjs design](https://github.com/ngolacloud/dxjs/blob/main/DESIGN.md) for the
  reactivity model, SSR/hydration and the transport in full.

---

*Part of the [Seiva documentation](../../index.md).*

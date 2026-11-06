// A separate dxjs storefront for the shop API (tutorial Part 11), now with
// client-side routing: a catalogue view and a product-detail view, swapped by
// `@dxjs/component`'s History-API router. It runs on its own origin and talks to
// the Seiva `shop-api` cross-origin over CORS. No business logic lives here; the
// server stays the single authority.
//
// Built with esbuild from the sibling dxjs packages (see build.mjs). Point it at
// the API with `window.SHOP_API_BASE` (default :8090).

import { signal } from "@dxjs/core";
import { html } from "@dxjs/dom";
import { component, createRouter } from "@dxjs/component";
import { createClient, resource } from "@dxjs/transport";
import type { RouteContext } from "@dxjs/component";

const API: string =
  (globalThis as { SHOP_API_BASE?: string }).SHOP_API_BASE ?? "http://127.0.0.1:8090";

interface ProductDto {
  id: string;
  name: string;
  price_cents: number;
}

const client = createClient({ baseUrl: API });

// Shared across views: the bearer token (the demo shop-api accepts `shop-token`),
// in the Authorization header only — never a URL or a cookie.
const token = signal("shop-token");
const auth = () => ({ authorization: "Bearer " + token() });
const money = (cents: number) => (cents / 100).toFixed(2);

// --- Catalogue view (route "/") ---------------------------------------------
const Catalogue = component(() => {
  const products = resource<ProductDto[]>((sig) =>
    client.get<ProductDto[]>("/v1/catalogs", { signal: sig, headers: auth() }),
  );
  const status = signal("");
  const name = signal("");
  const price = signal("1000");

  const addProduct = (ev: Event): void => {
    ev.preventDefault();
    status.set("adding…");
    client
      .post("/v1/catalogs/add_product", { name: name(), price_cents: Number(price()) }, { headers: auth() })
      .then(() => {
        status.set("added");
        name.set("");
        products.refetch();
      })
      .catch((e: unknown) => status.set("add failed: " + String(e)));
  };

  return html`
    <main>
      <h1>Shop</h1>
      <p class="api">API: ${API} <span class="status">${() => status()}</span></p>

      <label>Bearer token
        <input value=${() => token()} @input=${(e: Event) => token.set((e.target as HTMLInputElement).value)} />
      </label>

      <h2>Catalogue</h2>
      ${() => (products.loading() ? html`<p>Loading…</p>` : null)}
      ${() => (products.error() ? html`<p class="err">Could not load the catalogue.</p>` : null)}
      <ul class="products">
        ${() =>
          (products() ?? []).map(
            (p) => html`
              <li>
                <a class="name" href=${"/product/" + p.id} @click=${(e: Event) => { e.preventDefault(); router.navigate("/product/" + p.id); }}>${p.name}</a>
                <span class="price">${money(p.price_cents)}</span>
              </li>`,
          )}
      </ul>

      <h2>Add a product</h2>
      <form @submit=${addProduct}>
        <input placeholder="name" value=${() => name()} @input=${(e: Event) => name.set((e.target as HTMLInputElement).value)} />
        <input placeholder="price (cents)" value=${() => price()} @input=${(e: Event) => price.set((e.target as HTMLInputElement).value)} />
        <button type="submit">Add</button>
      </form>
    </main>`;
});

// --- Product-detail view (route "/product/:id") -----------------------------
const ProductView = component((ctx: RouteContext) => {
  const id = ctx.params.id;
  const product = resource<ProductDto>((sig) =>
    client.get<ProductDto>("/v1/catalogs/" + id, { signal: sig, headers: auth() }),
  );
  const status = signal("");

  const order = (): void => {
    status.set("placing order…");
    client
      .post(
        "/v1/orders/place",
        { customer: "web", lines: [{ product_id: Number(id), qty: 1 }] },
        { headers: auth() },
      )
      .then(() => status.set("order placed ✓"))
      .catch((e: unknown) => status.set("order failed: " + String(e)));
  };

  return html`
    <main>
      <p><a href="/" @click=${(e: Event) => { e.preventDefault(); router.navigate("/"); }}>← Catalogue</a></p>
      ${() => {
        const p = product();
        if (product.loading()) return html`<p>Loading…</p>`;
        if (!p) return html`<p class="err">Product not found.</p>`;
        return html`
          <h1>${p.name}</h1>
          <p class="price">Price: ${money(p.price_cents)}</p>
          <button @click=${order}>Order one</button>
          <p class="status">${() => status()}</p>`;
      }}
    </main>`;
});

const router = createRouter(
  [
    { path: "/", component: Catalogue },
    { path: "/product/:id", component: ProductView },
  ],
  { fallback: "/" },
);

router.outlet(document.querySelector("#app")!);

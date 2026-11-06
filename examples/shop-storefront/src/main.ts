// A separate dxjs storefront for the shop API (tutorial Part 11). It runs on its
// own origin and talks to the Seiva `shop-api` cross-origin over CORS — browse
// the catalogue, add a product, and place an order. No business logic lives here;
// the server stays the single authority.
//
// Built with esbuild from the sibling dxjs packages (see build.mjs); runs in any
// browser. Point it at the API with `window.SHOP_API_BASE` (default :8090).

import { signal } from "@dxjs/core";
import { html } from "@dxjs/dom";
import { component, mount } from "@dxjs/component";
import { createClient, resource } from "@dxjs/transport";

const API: string =
  (globalThis as { SHOP_API_BASE?: string }).SHOP_API_BASE ?? "http://127.0.0.1:8090";

interface ProductDto {
  id: string;
  name: string;
  price_cents: number;
}

const Storefront = component(() => {
  // The bearer token (the demo shop-api accepts `shop-token`); rides only in the
  // Authorization header, never in a URL.
  const token = signal("shop-token");
  const status = signal("");
  const auth = () => ({ authorization: "Bearer " + token() });
  const client = createClient({ baseUrl: API });

  const products = resource<ProductDto[]>((sig) =>
    client.get<ProductDto[]>("/v1/catalogs", { signal: sig, headers: auth() }),
  );

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

  const order = (productId: string): void => {
    status.set("ordering…");
    client
      .post(
        "/v1/orders/place",
        { customer: "web", lines: [{ product_id: Number(productId), qty: 1 }] },
        { headers: auth() },
      )
      .then(() => status.set("order placed"))
      .catch((e: unknown) => status.set("order failed: " + String(e)));
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
                <span class="name">${p.name}</span>
                <span class="price">${(p.price_cents / 100).toFixed(2)}</span>
                <button @click=${() => order(p.id)}>Order one</button>
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

mount(Storefront, document.querySelector("#app")!, {});

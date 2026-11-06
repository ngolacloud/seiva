# Parte 8: Um frontend de browser com dxjs

A loja é um backend completo — um serviço contract-first (ou uma frota de
serviços, depois da Parte 7) com OpenAPI e permissões derivadas da fachada de cada
módulo. Esta parte põe-lhe uma **montra** com
[**dxjs**](https://github.com/ngolacloud/dxjs), o motor reactivo de UI da Delonix:
percorrer o catálogo e colocar uma encomenda, a falar com a API da loja que
construíste.

!!! note "dxjs é o motor de frontend; `dx` é o gestor de pacotes"
    Não confundas as duas ferramentas Delonix. O [`dx`](../../dx/index.md) compila
    e corre o teu projecto Rust. O **dxjs** é uma biblioteca de UI em TypeScript
    (signals, sem virtual DOM) que renderiza no browser.

O dxjs usa **signals** para actualizações cirúrgicas do DOM (uma app reactiva
completa tem ~2,8 KB gzip) e renderiza sem `eval` e sem handlers inline, por isso
corre sob uma Content-Security-Policy estrita.

## Os pacotes e um cliente

```ts
import { signal } from "@dxjs/core";
import { html } from "@dxjs/dom";
import { component, mount } from "@dxjs/component";
import { createClient, resource } from "@dxjs/transport";

const client = createClient({ baseUrl: "" }); // mesma origem que a loja
```

Espelha os DTOs da loja — os formatos que a Parte 2 publicou em `/openapi.json`:

```ts
interface ProductDto { id: string; name: string; price_cents: number }
interface LineDto    { product_id: string; qty: number }
interface OrderDto   { id: string; customer: string; lines: LineDto[]; status: string }
```

## Percorrer o catálogo

O `resource` transforma um fetch assíncrono em signals reactivos — o valor,
`loading()` e `error()`. Este é o caso de uso `catalog.list` no browser:

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

Cada binding `${() => …}` é uma actualização cirúrgica, e os valores interpolados
passam por sinks escapados — um produto chamado `<script>…` aparece como texto,
nunca é executado.

## Colocar uma encomenda

Um pequeno signal de carrinho junta as linhas; colocar a encomenda é um só `POST`
ao `orders.place` (precisa da permissão `orders.manage` — Parte 5). O token viaja
apenas no cabeçalho `Authorization`:

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
    // O backend corre agora todo o fluxo da Parte 3: a encomenda emite
    // `orders.placed`, que desencadeia a reserva, a cobrança e a confirmação —
    // nada disso tem o frontend de orquestrar.
    cart.set([]);
    return order;
  });
```

O frontend não guarda regras de negócio: lê o catálogo e coloca uma encomenda
através do contrato, e o servidor corre o fluxo da encomenda e continua a ser a
única autoridade. Que o `payments` esteja no monólito ou separado (Parte 7) é
indiferente para este código.

## Montá-lo

```ts
mount(Catalogue, document.querySelector("#app"), {});
```

O `mount` devolve um disposer que desmonta todas as subscrições. Para navegação
multi-vista (catálogo → confirmação da encomenda), o `@dxjs/component` traz o
`createRouter`; para um estado de encomenda ao vivo que se actualiza à medida que o
backend emite eventos, o `createLiveSource` do `@dxjs/transport` subscreve um fluxo
Server-Sent-Events.

## Construíste uma loja — da frente ao fundo

Um backend de cinco módulos que corre como monólito ou como microserviços, e uma
montra reactiva minúscula e segura sob CSP que consome exactamente o seu contrato.
Para onde ir a seguir:

- Os [guias temáticos](../../topics/index.md) para aprofundar cada camada.
- A [secção DORM](../../dorm/index.md) e a [secção dx](../../dx/index.md).
- O [desenho do dxjs](https://github.com/ngolacloud/dxjs/blob/main/DESIGN.md) para
  o modelo de reactividade, SSR/hidratação e o transporte na íntegra.

---

*Parte da [documentação do Seiva](../../index.md).*

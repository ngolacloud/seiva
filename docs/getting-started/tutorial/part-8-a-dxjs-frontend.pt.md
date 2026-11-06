# Parte 8: Um frontend no browser com dxjs

Na Parte 7 o poll-site é um backend completo: um serviço Seiva contract-first que
serve `GET /v1/polls`, `GET /v1/polls/{id}`, `POST /v1/polls/vote` e os resultados,
com OpenAPI e permissões derivados da fachada. Esta parte põe-lhe uma **UI de
browser** com [**dxjs**](https://github.com/ngolacloud/dxjs) — o motor de UI
reactivo de granularidade fina da Delonix — para que o poll-site se torne
full-stack: perguntas, opções, votação e resultados ao vivo no browser, a falar com
a API Seiva que já construíste.

!!! note "o dxjs é o motor de frontend; o `dx` é o gestor de pacotes"
    Não confundas as duas ferramentas Delonix. O [`dx`](../../dx/index.md) constrói
    e corre o teu projecto Rust. O **dxjs** é uma biblioteca de UI em TypeScript
    (signals, sem virtual DOM) que renderiza no browser. São independentes para
    além do nome de família.

O dxjs usa **signals** para actualizações cirúrgicas do DOM (uma app reactiva
completa são ~2,8 KB gzip), e renderiza sem `eval` e sem handlers inline, por isso
corre sob uma Content-Security-Policy estrita — a mesma postura de que o admin da
consola Seiva precisa.

## Os pacotes

Uma app dxjs puxa três pacotes pequenos, mais o cliente de transporte:

```ts
import { signal } from "@dxjs/core";          // reactivity
import { html } from "@dxjs/dom";             // escaped templates
import { component, mount } from "@dxjs/component";
import { createClient, resource } from "@dxjs/transport"; // REST to the Seiva API
```

O dxjs corre TypeScript directamente em Node ≥ 22 (type-stripping nativo), por isso
não há passo de build para o ver a funcionar.

## Um cliente para a API Seiva

`createClient` envolve o `fetch` para uma origem. Aponta-o para o serviço Seiva em
execução (mesma origem em produção, ou o endereço de dev):

```ts
const client = createClient({ baseUrl: "" }); // same origin as the page
```

Espelha os DTOs da tua fachada como tipos TypeScript — as mesmas formas que a Parte
2 publicou em `/openapi.json`, para que o contrato de wire seja a única fonte de
verdade:

```ts
interface QuestionDto { id: string; text: string }
interface ChoiceDto   { id: string; question_id: string; text: string; votes: number }
interface QuestionDetail { question: QuestionDto; choices: ChoiceDto[] }
```

## A vista de índice — perguntas mais recentes

`resource` transforma um fetch assíncrono em três signals — o valor, `loading()` e
`error()` — mais `refetch()`. É o equivalente reactivo do caso de uso `list` da
Parte 1:

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

Cada binding `${() => …}` é uma actualização cirúrgica: quando um signal que lê
muda, só esse nó é corrigido — sem re-render, sem virtual DOM. Os valores
interpolados (`q.text`) passam por sinks com escape, por isso uma pergunta
intitulada `<script>…` é mostrada como texto, nunca executada.

## Detalhe, votação e resultados

`GET /v1/polls/{id}` devolve a pergunta com as suas opções; `POST /v1/polls/vote`
incrementa uma contagem (precisa da permissão `polls.vote` — Parte 4). Depois de um
voto bem-sucedido fazemos `refetch()` para que a contagem actualize no lugar:

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

O token bearer viaja apenas no cabeçalho `Authorization`, nunca num URL — a mesma
regra que o backend espera. A ocultação ao estilo `can` do botão de voto para um
chamador só-de-leitura é apresentacional; o controlador Seiva continua a impor
`polls.vote` do lado do servidor, por isso a UI não pode conceder um acesso que o
contrato nega.

## Montá-lo

```ts
mount(Questions, document.querySelector("#app"), {});
```

`mount` devolve um disposer que desmonta todas as subscrições de signals e bindings
de DOM — não há fuga quando uma vista é substituída. Para navegação multi-vista
(`#/q/{id}` → a vista de detalhe), `@dxjs/component` traz `createRouter`.

## Resultados ao vivo (opcional)

Se o serviço emitir os seus eventos `polls.voted` por Server-Sent Events, o
`createLiveSource` de `@dxjs/transport` subscreve-os e deixa os resultados
actualizarem-se sem polling — a contagem sobe à medida que outras pessoas votam. O
caminho REST acima funciona sem streaming; liga a fonte ao vivo apenas quando o
backend publicar um endpoint de eventos.

## O poll-site é full-stack

Tens agora o ciclo todo: um backend Seiva compilado cujo contrato comanda as rotas,
o OpenAPI e as permissões (Partes 1–6), o mesmo módulo promovível ao seu próprio
serviço (Parte 7), e uma UI reactiva minúscula e segura em CSP (esta parte) que
consome exactamente esse contrato. O frontend não contém regras de negócio — lê e
escreve através da API, e o servidor permanece a autoridade única.

Para onde a seguir:

- Os [guias temáticos](../../topics/index.md) para profundidade em cada camada do backend.
- A [secção DORM](../../dorm/index.md) para levar a camada de dados mais longe.
- O [desenho do dxjs](https://github.com/ngolacloud/dxjs/blob/main/DESIGN.md) para o
  modelo de reactividade, SSR/hidratação e o transporte por inteiro.

---

*Parte da [documentação do Seiva](../../index.md).*

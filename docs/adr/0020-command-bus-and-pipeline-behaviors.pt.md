# ADR 0020 — O bus de comandos/queries liga-se ao pipeline com behaviors componíveis (opt-in)

- Status: Aceite
- Data: 2026-11-08
- Decisores: NgolaCloud
- Segue: ADR 0002 (ports), ADR 0003 (camadas de módulo), ADR 0006 (identidade), ADR 0011 (idempotência durável), ADR 0019 (autorização fail-closed)

## Contexto

O Seiva já traz um núcleo de dispatch CQRS. O `seiva-cqrs` tem mensagens tipadas
`Command`/`Query`, casos de uso `CommandHandler`/`QueryHandler`, e um `Mediator`
cujo dispatch é **estaticamente tipado e zero-cost**: é genérico sobre o handler
que guarda, logo `send`/`query` resolvem numa chamada directa, monomorfizada, sem
boxing e sem procura por `TypeId` — um handler em falta é um *erro de tipo*, não um
panic em runtime (`crates/seiva-cqrs/src/lib.rs`). Demonstra até o seam de
middleware com o `Instrument` (conta dispatches, o gancho do `#[policy]`) e o
`CacheQueries` (memoiza uma query do lado-leitura, o gancho do `#[cached]`),
compostos por aninhamento: `Mediator::new(Instrument::new(CacheQueries::new(handler)))`.

Mas o núcleo **não está no caminho do pedido**. Duas lacunas:

- **O bus está desligado do transporte.** O `#[seiva::controller]` gera um router
  cujos handlers chamam o método do facade/service *directamente*
  (`self.0.place(cmd).await`, `examples/shop/orders/interface/src/lib.rs`); o
  `Mediator` só aparece nos *testes* desse módulo. Nenhum pedido de produção o
  atravessa.
- **As cross-cutting concerns estão espalhadas.** Authn e rate-limit são camadas
  tower (`seiva-http`), a authz é um guard por-rota montado pela macro (ADR 0019),
  a idempotência é conduzida pela aplicação *dentro* da sua transacção (ADR 0011),
  a validação é o extractor `Valid<T>`. Cada uma está correcta, mas não há um único
  sítio tipado onde se expresse, por caso de uso, «o que tem de acontecer à volta
  de uma escrita» — e os seams `#[policy]`/`#[cached]` do facade são reconhecidos
  pela macro mas **não produzem comportamento** (`seiva-macros/src/lib.rs`).

As forças:

- Dar a um caso de uso **um pipeline tipado** para as suas cross-cutting concerns,
  sem abdicar do dispatch compile-time e zero-cost em que o Seiva assenta.
- Respeitar um facto estrutural duro: **a idempotência e a escrita no outbox têm de
  viver *dentro* da transacção** (o ADR 0011 rejeitou um middleware exterior
  precisamente porque corre acima de qualquer transacção e não consegue manter-se
  atómico com a mutação). Logo, o pipeline não pode ser uma pilha plana de camadas
  todas acima do handler.
- Não reescrever o caminho HTTP que acabou de ser endurecido e enverdecido (PR #29).
  A mudança tem de ser reversível e adoptável um caso de uso de cada vez.
- Ser agnóstico ao transporte: um controller HTTP, um serviço gRPC e um consumidor
  de eventos deviam partilhar as mesmas behaviors, já que a preocupação de negócio
  é a mesma.

## Decisão

**Introduzir um pipeline tipado de *behaviors* à volta do `Mediator` existente, e
deixar que um caso de uso adira a ele por comando. A macro controller continua a
chamar o service directamente por default; nada é forçado pelo bus.** Uma behavior
é ela própria um handler que embrulha um handler interior — exactamente a forma que
o `Instrument` e o `CacheQueries` já usam — por isso as behaviors compõem por
aninhamento e mantêm-se totalmente tipadas e monomorfizadas. Sem registo em
runtime, sem `TypeId → Box<dyn Any>`.

As behaviors entregues, e onde se situam face ao limite da transacção:

- **Pré-transacção (falhar cedo, antes de tocar na base de dados):** `Authorize`
  (reutiliza a política `#[requires]`/`#[public]` do descriptor, ADR 0019),
  `Validate` (o contrato `Valid<T>`/`Validate`), `Trace` (o seam do `Instrument` /
  um span `tracing`).
- **Âmbito transaccional:** uma behavior `Transactional` abre
  `UnitOfWork::transaction` (ADR 0002/0004 — baseada em âmbito, closure-scoped) e
  corre a cadeia interior *dentro* dela. Como a unidade de trabalho é um âmbito, a
  behavior recebe o resto da cadeia como corpo da closure, pelo que os passos
  seguintes confirmam (commit) genuinamente juntos:
  - **Intra-transacção:** `Idempotent` (`IdempotencyStore::begin`/`finish`,
    ADR 0011), o **handler** do caso de uso (a mutação), e o **publish no outbox**
    (`EventPublisher`, escrito na mesma transacção, ADR 0011/CLAUDE.md).

Esta separação é o cerne da questão: o bus não achata tudo numa pilha de
middlewares acima do handler — modela que algumas behaviors estão *fora* da
transacção (rejeitam cedo, barato) e outras estão *dentro* dela (atómicas com a
mutação). A ordem para uma escrita é, portanto:

```text
Trace → Authorize → Validate → Transactional{ Idempotent → handler → outbox }
```

Uma leitura (`Query`) usa o subconjunto `Trace → Authorize → CacheQueries →
handler`; comandos nunca são cacheados (o `CacheQueries` já deixa os comandos
passar intocados).

**O opt-in é o mecanismo de adopção.** Um caso de uso entra no bus ligando o seu
handler à cadeia de behaviors na raiz de composição; um caso de uso não migrado
continua a chamar o seu service directamente, tal como hoje. Os seams `#[cached]` e
`#[policy]` do facade passam a estar vivos — seleccionam as behaviors
`CacheQueries` / `Trace`+`Authorize` para esse método — em vez de serem marcadores
inertes.

Isto activa, enfim, os seams que o `seiva-cqrs` foi escrito para expor e fecha a
lacuna que a própria documentação do seu módulo descreve («o gancho
`#[policy]`/`#[cached]` vai mapear para isto»).

## Alternativas consideradas

- **Tornar o bus o default no `#[seiva::controller]` (encaminhar todos os casos de
  uso por ele).** A consolidação máxima, e o provável estado final. Rejeitada *por
  agora*: reescreve todo o caminho HTTP provado de uma só vez, logo a seguir a uma
  passagem de estabilização, que é exactamente por onde as regressões voltam a
  entrar. O plano é provar o pipeline num módulo (orders) sob opt-in, e depois
  reavaliar um default num ADR de seguimento quando a maioria dos casos de uso já lá
  correr.
- **Um registo de mediator em runtime (`TypeId → Box<dyn Any>`, à la MediatR).**
  Rejeitada, e a própria documentação do `seiva-cqrs` já argumenta porquê:
  reintroduz um modo de falha em runtime que o sistema de tipos de outra forma
  exclui, obriga a boxing e dispatch dinâmico, e torna a composição de middleware
  não-tipada. Contraria o princípio «tudo conhecido em compile-time» do framework.
- **Manter as preocupações como camadas tower HTTP (não fazer nada).** Rejeitada: o
  tower opera sobre o `Request`/`Response` HTTP, *acima* de qualquer transacção,
  logo não consegue alojar as behaviors de idempotência/outbox que têm de ser
  atómicas com a mutação — a mesma razão por que o ADR 0011 evitou um middleware de
  idempotência exterior. Também não é agnóstico ao transporte: uma chamada gRPC ou
  um consumidor de eventos nunca passa pela pilha tower HTTP.
- **Uma nova macro `#[seiva::command]` que gera o handler + a ligação.** Uma camada
  ergonómica razoável, mas cristaliza a ligação antes de o pipeline estar provado.
  Adiada: começar com ligação explícita na raiz de composição; acrescentar uma
  macro só quando o conjunto e a ordem das behaviors estabilizarem.

## Consequências

- **Fica mais fácil:** um caso de uso declara as suas cross-cutting concerns como
  uma cadeia única, tipada e monomorfizada; os seams `#[cached]`/`#[policy]`
  finalmente fazem alguma coisa; as mesmas behaviors servem HTTP, gRPC e
  consumidores de eventos por ficarem abaixo do transporte.
- **Fica mais difícil / dívida (datada 2026-11-08):** durante a migração coexistem
  dois caminhos — um caso de uso vai pelo bus ou chama o seu service directamente —
  por isso um leitor tem de saber qual. Aceite deliberadamente para evitar uma
  reescrita big-bang; o opt-in é o preço da reversibilidade. A ordem das behaviors
  face ao limite da transacção (pré- vs intra-transacção) é uma invariante de
  correcção que tem de ser documentada e testada, não deixada à convenção.
- **Imposição:** um exemplo (orders) é migrado para o bus como prova, com um teste
  que exercita a cadeia completa (authz → validação → idempotência → tx → outbox) e
  mostra um só efeito sob uma chave de idempotência repetida; um check de fitness de
  que o dispatch se mantém estático (sem `dyn`/`Any` no caminho do caso de uso); os
  exemplos de chamada directa existentes ficam verdes, provando que o opt-in não os
  perturba.

## Relacionado

- [ADR 0002](0002-core-ports-and-test-doubles.md) · [ADR 0003](0003-module-layering-and-facade-impl.md) · [ADR 0006](0006-identity-and-authentication.md) · [ADR 0011](0011-durable-idempotency.md) · [ADR 0019](0019-fail-closed-authorization.md) · [ADR 0021](0021-in-process-event-bus-and-choreography.md)

---

*Parte da [documentação do Seiva](../index.md).*

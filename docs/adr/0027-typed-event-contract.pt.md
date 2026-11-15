# ADR 0027 — Um evento de domínio é um contrato publicado: um evento tipado, versionado e com esquema, em vez de um blob JSON ad-hoc

- Status: Proposta
- Data: 2026-11-15
- Decisores: NgolaCloud
- Segue: ADR 0007 (fidelidade de contrato WireSchema), ADR 0021 (bus de eventos in-process e coreografia), ADR 0003 (camadas do módulo)

## Contexto

A promessa central do Seiva é que os **módulos cooperam por eventos, não por
chamadas** (CLAUDE.md, ADR 0021). A metade síncrona dessa cooperação — a facade —
é um contrato de primeira classe: um trait `#[seiva::facade]` cujos DTOs derivam
`WireSchema` e são publicados no OpenAPI, versionados, guardados por permissão, a
fonte única de verdade (ADR 0007). Quem consome uma facade compila contra um
contrato tipado e descobrível.

A metade assíncrona — o evento — **não tem contrato nenhum**. O scaffold ensina o
developer a publicar assim (`scaffold.rs` `application_lib`):

```rust
let event = serde_json::to_vec(&serde_json::json!({
    "id": record.id.to_string(),
    "title": record.title.clone(),
}))?;
self.events.publish("notes.record.created", &event).await
```

O tópico é uma string nua; o payload é um objecto JSON anónimo construído à mão
dentro do caso de uso. Do lado consumidor (`scaffold.rs` `event_main_rs`) um
subscritor recebe um `CloudEvent` e lê `event.ty` como string, depois tem de fazer
`serde_json::from_slice` do corpo para *alguma* forma que espera que o produtor
emita. A própria ADR 0021 nomeia isto: o dispatcher «deserialises each CloudEvent
into the subscriber's concrete type» — mas não há tipo do lado do produtor que
garanta que os bytes correspondem a esse tipo concreto.

Ou seja, o canal que o framework declara como o acoplamento **principal** entre
módulos é o único sem linguagem publicada. As forças que isto cria:

- Um módulo produtor muda `title` para `name`; nada deixa de compilar. Todos os
  consumidores partem em runtime, no evento seguinte, noutro processo — exactamente
  o modo de falha que a facade tipada foi construída para evitar.
- Um consumidor não tem de onde gerar documentação, uma entrada de registo de
  esquemas, ou um mock. O evento é indescobrível: `seiva openapi` descreve cada
  rota mas nenhum evento.
- O scaffold é o código mais copiado do framework (o tema recorrente da revisão de
  arquitectura). Ensina envelopes `serde_json::json!` ad-hoc como norma, por isso
  cada módulo real reinventa uma forma de evento sem versão e sem validação.

É o achado **A2** (Alto) em `docs/audits/2026-11-architecture-review.md`. É uma
lacuna de Published Language / Context Mapping (Fowler) e uma lacuna de modelo de
domínio (Evans — o evento de domínio é um conceito de primeira classe, não um
literal de dicionário).

A restrição que a solução tem de respeitar: a ADR 0021 fixou o **transporte** dos
eventos (o outbox transaccional, CloudEvents 1.0 no fio, at-least-once com dedup
pelo inbox, o mesmo código de subscritor em monólito e após um split). A ADR 0027
tem de acrescentar um **contrato** sobre esse transporte **sem** mudar o transporte
nem a regra de camadas (ADR 0003): o tipo do evento tem de viver onde vive um DTO
de facade (a camada de contrato), ser neutro quanto ao transporte, e não obrigar um
consumidor a importar os crates internos do produtor.

## Decisão

**Um evento de domínio é declarado como um contrato tipado, versionado e com
esquema, no crate `facade` do módulo que o publica — a mesma camada, e a mesma
disciplina de contrato, que os seus DTOs de pedido/resposta.** O evento é a
Published Language da fronteira assíncrona do módulo.

1. **Um payload de evento tipado com `WireSchema`.** O módulo produtor declara o
   corpo do evento como uma struct no seu crate `facade`, derivando o mesmo
   `serde` + `WireSchema` de um DTO de resposta, para que os seus campos sejam
   descobríveis e publicáveis exactamente como um DTO de facade (ADR 0007):

   ```rust
   // in <module>-facade
   #[derive(Debug, Clone, Serialize, Deserialize, WireSchema)]
   pub struct RecordCreated {
       pub id: String,
       pub title: String,
   }
   ```

   O tipo vive em `facade` porque faz parte do contrato público do módulo, não dos
   seus internos de domínio — um consumidor pode nomeá-lo sem alcançar o
   `domain`/`application` do produtor, preservando as camadas da ADR 0003 e a
   independência de que o `module split` depende.

2. **Um tópico versionado e declarado.** O evento declara a sua string de tipo
   **uma vez**, junto do payload, com um sufixo de versão explícito —
   `notes.record.created.v1` — em vez de uma string livre escrita no sítio da
   chamada a `publish`. Um macro ou uma const no tipo é a fonte única dessa string,
   para que o produtor e o `ty` do CloudEvent não possam divergir. A versão faz
   parte do contrato: uma mudança incompatível do payload é `…​.v2`, publicada a par
   de `…​.v1` até os consumidores migrarem — o gémeo assíncrono da versão de facade.

3. **Um registo de eventos em link-time, espelhando o registo de facades.** Cada
   tipo de evento é registado em link-time (como as facades e as seeds já são —
   `registered_facades`, `registered_seeds`), para que `seiva openapi` (ou um novo
   `seiva events`) possa emitir o catálogo de eventos que um binário produz, e uma
   fitness test possa afirmar que os bytes emitidos pelo scaffold correspondem ao
   esquema registado. Os eventos tornam-se tão descobríveis como as rotas.

4. **A publicação e o transporte ficam inalterados (a ADR 0021 mantém-se).** O caso
   de uso continua a publicar **dentro** da sua `UnitOfWork::transaction` pelo port
   `EventPublisher`, continua para o outbox transaccional, continua CloudEvents 1.0
   no fio, continua at-least-once com dedup pelo inbox. A única mudança é que o
   payload que serializa é um valor tipado (`serde_json::to_vec(&RecordCreated { … })`)
   sob um tópico declarado, não um literal `json!` construído à mão sob uma string. O
   `from_slice::<RecordCreated>` do subscritor passa a ter uma garantia do lado do
   produtor por trás.

5. **O scaffold ensina a forma tipada.** O `scaffold.rs` gera o tipo de evento
   `RecordCreated` na facade, publica-o tipado na application, e (para `--kind event`)
   deserializa-o tipado no subscritor — para que o código mais copiado modele o
   contrato, não o blob ad-hoc. É isto que fecha A2: a fitness
   `generated_event_matches_its_registered_schema` fica verde.

## Alternativas consideradas

- **Deixar o evento como JSON ad-hoc, documentar uma convenção.** Rejeitado: é o
  status quo que A2 nomeia. Uma convenção que o compilador não impõe é reinventada
  (e partida) por módulo; o framework já rejeitou isto para o contrato síncrono
  (ADR 0007) e não deve ter dois pesos e duas medidas para o canal a que chama
  principal.
- **Pôr o tipo do evento no `domain` (o evento é um conceito de domínio — Evans).**
  O evento *origina-se* no domínio, mas colocar o tipo publicado no `domain`
  obrigaria um consumidor a depender do `domain` do produtor para o nomear, violando
  a ADR 0003 e a independência de que o `module split` precisa. Mantemos o **evento
  de domínio** como conceito de domínio internamente e expomos a sua forma
  **publicada** como contrato da camada facade — a mesma divisão que a facade já faz
  entre uma entidade de domínio e o seu DTO de fio.
- **Um crate `events` central e partilhado que todos os módulos importam.**
  Rejeitado: recria o acoplamento em tempo de compilação entre módulos que os
  eventos existem para evitar — um consumidor acoplar-se-ia transitivamente a todos
  os produtores. Cada evento é propriedade da facade do módulo que o produz,
  descoberto em link-time, nunca por importação entre módulos.
- **Gerar o esquema do evento a partir da string do tópico em runtime (reflexão).**
  Rejeitado: o contrato do Seiva é em tempo de compilação sem reflexão (CLAUDE.md);
  o catálogo de eventos tem de ser conhecido estaticamente como o documento OpenAPI.
- **Um registo de esquemas externo completo (registo Avro/Protobuf).** Adiado, não
  rejeitado: o envelope de fio CloudEvents poderia mais tarde carregar um
  `dataschema` a apontar para um JSON Schema publicado derivado do `WireSchema`. A
  ADR 0027 estabelece primeiro o contrato tipado no repo e o catálogo em link-time;
  ligar um registo remoto é um passo posterior e mais estreito.

## Consequências

- **Mais fácil:** um consumidor compila contra um evento tipado; uma mudança
  incompatível do payload é um `.v2` visível, não uma quebra silenciosa em runtime;
  os eventos são descobríveis (`seiva events` / o catálogo) como as rotas são; o
  scaffold ensina o contrato, por isso os módulos novos herdam-no em vez de
  reinventar um blob.
- **Mais difícil / dívida (datada 2026-11-15):** um registo de eventos em link-time
  e a canalização do `WireSchema`-do-evento são código novo (parente próximo do
  registo de facades, mas novo). Os módulos de exemplo escritos à mão e quaisquer
  eventos `json!` ad-hoc em `apps/*` têm de migrar para a forma tipada — uma mudança
  mecânica e rastreada, como a migração dos exemplos de F4. A versão de tópico
  acrescenta uma disciplina `.vN` que os produtores têm de seguir; o framework deve
  avisar sobre uma string de tópico que não carregue versão.
- **Imposição (as fitness functions que fecham A2):**
  `generated_event_matches_its_registered_schema` — os bytes que a application do
  scaffold publica deserializam para o tipo de evento `WireSchema` registado;
  `event_catalogue_lists_the_module_event` — os eventos registados de um binário são
  emitidos por `seiva events`/o catálogo; e o e2e de coreografia existente continua a
  provar um efeito por evento, agora sobre um payload tipado.

## Relacionadas

- [ADR 0003](0003-module-layering-and-facade-impl.md) · [ADR 0007](0007-wire-schema-contract-fidelity.md) · [ADR 0011](0011-durable-idempotency.md) · [ADR 0021](0021-in-process-event-bus-and-choreography.md)
- Achado **A2** no [backlog da revisão de arquitectura](../audits/2026-11-architecture-review.md).

---

*Parte da [documentação do Seiva](../index.md).*

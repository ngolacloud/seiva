# ADR 0003 — Estratificação do módulo e onde a fachada é implementada

- Estado: Aceite
- Data: 2026-11-03
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0002

## Contexto

Um módulo tem cinco camadas, uma por crate, com a regra de dependência imposta
pelo grafo de crates (brief, Parte A). O brief fixa aquilo de que cada camada
pode depender mas deixa um detalhe para "a ADR do núcleo": o trait da fachada é
o contrato **de entrada** do módulo (`#[seiva::facade]`), mas tem de ser
implementado algures, e essa implementação tem de mapear o resultado de um
caso de uso (uma entidade de domínio) para o DTO da fachada. Que camada detém
essa implementação decide toda a ligação.

A restrição que força a resposta: `interface` não pode depender de `domain`
(tabela do brief), por isso `interface` não pode mapear uma entidade de domínio
para um DTO. E `facade` só pode depender de `shared/kernel` e das macros do
Seiva, por isso não se pode implementar a si própria. A tabela do brief, porém,
permite explicitamente que `application` dependa dos **"DTOs da própria
facade"** — a sua própria crate `facade`.

## Decisão

1. **O trait da fachada é implementado em `application`.** O serviço de
   aplicação (p. ex. `NotesService`) implementa o trait `#[seiva::facade]` do
   módulo, correndo o caso de uso e mapeando o resultado de domínio para o DTO da
   fachada. Esta é a "porta de entrada implementada pelo serviço de aplicação" do
   hexagonal. É permitido porque `application` pode depender da sua própria crate
   `facade` pelos DTOs e pelo trait.

2. **A regra de camadas é sobre dependências *directas*.** `application` depende
   directamente da sua crate `facade`, que por sua vez depende de `delonix-seiva`
   (a macro da fachada). Essa ligação transitiva não viola "application não pode
   depender de `delonix-seiva`": a regra proíbe o acoplamento *directo* ao
   runtime do framework, e `application` não nomeia nenhum item
   `delonix_seiva::`. A regra de camadas do `seiva lint` (uma fitness function
   posterior) verifica dependências directas.

3. **`interface` depende apenas de `application` + `facade`.** Guarda um valor do
   trait da fachada (`S: Notes`) e expõe-no sobre transporte (HTTP/gRPC, uma fase
   posterior). Nunca vê `domain`.

4. **A porta de repositório (`NoteRepository`) vive no `domain`** (DDD clássico —
   Evans: a interface de repositório faz parte da linguagem ubíqua do domínio),
   formulada num erro de **domínio** (`RepositoryError`), não no do framework. É
   implementada em `infrastructure`; a **application adapta** o `RepositoryError`
   ao `PortResult` do framework na fronteira do caso de uso. As portas transversais
   do *framework* (`UnitOfWork`, `EventPublisher`, `Clock`, `IdGenerator`, de
   `seiva-ports`) são técnicas, não conceitos de domínio, logo ficam na
   `application`. (Actualizado — reverte o original «porta na application» do achado
   F4: o repositório é uma abstracção de domínio; as portas técnicas são abstracções
   de aplicação.) A `application` **re-exporta** a porta de repositório do `domain`,
   logo o `interface` importa-a da `application` e continua a nunca nomear o
   `domain` — a regra de camadas (§3) mantém-se, e o domínio fica livre do framework
   (o `seiva lint` continua a passar sem mudar regra).

## Consequências

- O grafo de dependências de um módulo é: `facade → delonix-seiva, seiva-contract`;
  `domain →` (nada / `shared/kernel`); `application → domain, facade, seiva-ports`;
  `infrastructure → application, domain, seiva-ports`; `interface → application,
  facade`.
- **A facade é transporte-neutra (F1, fechado).** O `Validate`/`ValidationError` do
  DTO de escrita E o `Problem` RFC 9457 para onde o erro mapeia vivem em
  `seiva-contract` (sem axum; o `Problem` carrega um `http::StatusCode`, o tipo de
  estado ao nível do protocolo que HTTP e gRPC partilham). Logo a facade declara
  todo o seu contrato — DTOs, validação, erro — **sem nomear nenhum crate de
  transporte**. O transporte HTTP embrulha um `Problem` num
  `seiva_http::ProblemResponse` (a borda `IntoResponse` do axum) para render
  `application/problem+json`; outro transporte mapeia o mesmo `Problem` à sua
  maneira. O macro `#[controller]` exige que o erro da facade seja
  `Into<seiva_contract::Problem>`.
- **O único tipo de transporte que uma facade ainda pode nomear é o
  `seiva_http::Ctx`**, quando um caso de uso recebe o contexto do pedido (`&Ctx`:
  principal, inquilino, chave de idempotência). O `Ctx` é um conceito de
  transporte/pedido (lê cabeçalhos e partes do pedido via um extractor axum), logo
  fica no `seiva-http`; uma facade opta por ele só quando o caso de uso precisa do
  contexto. O módulo *gerado* não usa `&Ctx` e não nomeia transporte nenhum. (Mover
  os dados do `Ctx` para um crate neutro é um refinamento possível mais tarde, mais
  estreito que o contrato erro/validação que este ADR resolve.)
- Um caso de uso é testado construindo o serviço de aplicação com os duplos de
  `seiva-testing` mais um repositório falso — sem servidor, broker ou base de
  dados (ADR 0002), provado por `examples/notes`.
- A fronteira de erro: dentro da aplicação, as portas devolvem `PortResult`
  (`BoxError`); no trait da fachada o erro é mapeado para o tipo de erro do
  contrato, de modo que o detalhe interno não escape através da fronteira do
  módulo.

## Alternativas consideradas

- **Fachada implementada em `interface`.** Rejeitada: `interface` teria de mapear
  uma entidade de domínio para um DTO, o que exige depender de `domain` —
  proibido.
- **Uma crate de adaptador dedicada para a implementação da fachada.** Rejeitada
  por desnecessária: acrescenta uma sexta crate para aquilo que é naturalmente o
  trabalho do serviço de aplicação.
- **O caso de uso devolve o DTO directamente (sem entidade de domínio).**
  Rejeitada: o caso de uso trabalha na linguagem do domínio; mapear para o DTO é
  uma preocupação de fronteira, propriedade do serviço de aplicação, não uma
  razão para deixar escapar DTOs para o núcleo.

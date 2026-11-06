# Arquitectura

O Delonix Seiva é uma framework de backend assente numa ideia: **um módulo é dono
dos seus dados e expõe um único contrato legível por máquina.** Tudo o resto — as
rotas HTTP, o serviço gRPC, o documento OpenAPI, as verificações de permissões —
é derivado desse contrato. Este guia explica a estratificação, o contrato, e como
as peças se encaixam.

## A regra de dependência

Um módulo são cinco crates, e o compilador impõe a direcção das dependências — não
consegues violar a arquitectura e ainda assim compilar:

```
         interface
         /        \
  infrastructure   application
         \        /      \
          domain          facade
                            |
                        (seiva-ports, delonix-seiva)
```

- **`domain`** — entidades e regras de negócio. Não depende de nada (nem sequer da
  framework). Exemplo: `notes_domain::Note`, `accounts_domain::User` com as suas
  regras de username/credenciais.
- **`facade`** — o contrato público e versionado do módulo: um trait anotado com
  `#[seiva::facade]` mais os seus DTOs. Os DTOs não carregam segredos nem
  preocupações de persistência. Esta é a *única* coisa que outros módulos e
  transportes vêem.
- **`application`** — os casos de uso: um serviço que implementa o trait da
  fachada sobre **portas** de saída (um repositório que define, mais as portas
  transversais da framework). Sem transporte e sem camada de dados — não nomeia
  `axum`, nem SQL, nem DORM.
- **`infrastructure`** — adaptadores concretos para as portas da application (um
  repositório). O padrão é em memória e sem DORM; um adaptador assente em DORM vem
  de `seiva-data-dorm` por trás da feature `dorm`.
- **`interface`** — composição e ligação do transporte (o `#[seiva::controller]`
  vive aqui). Depende apenas da fachada no código de produção (ADR 0003); a
  composição do serviço concreto pertence à app que o corre.

Os módulos `examples/notes` e `examples/accounts` são a forma canónica e
exactamente o que `seiva module new` esqueletiza.

## O contrato de fachada

`#[seiva::facade(version = "v1")]` mantém o teu trait tal como escrito e gera um
tipo `<Trait>Facade` de tamanho zero cujo `DESCRIPTOR` é conhecido em tempo de
compilação:

```rust
pub struct FacadeDescriptor {
    pub name: &'static str,        // "Accounts"
    pub version: &'static str,     // "v1"
    pub methods: &'static [MethodDescriptor],
}
pub struct MethodDescriptor {
    pub name: &'static str,        // "create_user"
    pub requires: &'static str,    // "accounts.create"  (from #[requires(..)])
    pub params: &'static [Param],  // [{ name: "cmd", ty: "CreateUser" }]
    pub output: &'static str,      // "Result<UserDto, String>"
}
```

Não há reflexão nem registo em runtime: um método ou permissão em falta é um facto
de tempo de compilação. O equivalente escrito à mão daquilo que a macro gera está
documentado em `delonix_seiva::contract`.

## Transportes gerados a partir do descritor

Um contrato, três transportes — nenhum mantido à mão:

- **HTTP** (`seiva-http` + `#[seiva::controller]`): a macro gera
  `<trait>_router(service) -> axum::Router`, uma rota por método. A versão do
  caminho e a permissão exigida são lidas do descritor (nunca fixadas no código); a
  guarda rejeita um chamador sem a permissão com um `application/problem+json`
  RFC 9457. Os corpos de pedido são validados (`Valid<T>`).
- **gRPC** (`seiva-grpc`): `proto_document(&FacadeDescriptor) -> String` renderiza
  um serviço proto3 via um mapa documentado e auditável de tipo-Rust → tipo-proto;
  os parâmetros de contexto da framework não são campos de fio; `Result<T, E>` é
  desembrulhado para `T`. A feature opcional `tonic` mapeia um erro de fachada para
  `tonic::Status`.
- **OpenAPI** (`seiva-openapi`): `openapi_document(ApiInfo, &[FacadeDescriptor])`
  constrói um documento OpenAPI 3.1 válido — operações sob os seus caminhos
  versionados, `x-seiva-requires` e segurança bearer para métodos guardados,
  esquemas placeholder honestos para tipos opacos. Servido ao vivo em
  `/openapi.json`.

Os comandos de CLI `seiva openapi` e `seiva proto` imprimem estes para a fachada
de demonstração embutida hoje; um módulo gerado emite o seu próprio via a sua
camada de interface.

## As portas (`seiva-ports`)

A camada de application depende apenas deste crate só-de-traits, por isso é
testável sem servidor, broker ou base de dados (`seiva-testing` fornece dublês em
memória):

- `Clock` — a fonte de tempo.
- `IdGenerator` — ids estáveis de 16 bytes.
- `EventPublisher` — publicar um evento de domínio.
- `UnitOfWork` — **por escopo**: `transaction(|| async { ... })` corre o corpo
  numa só transacção. Com o gateway DORM isto mapeia para `with_session` +
  `db::atomic`, e o `EventPublisher` escreve para o **outbox transaccional** na
  *mesma* transacção, por isso a mudança de negócio e o evento fazem commit juntos.
  Um worker de relay reencaminha depois o evento para o broker.

## Despacho de comandos/consultas (`seiva-cqrs`)

O barramento interno de aplicação: traits `Command`/`Query`, os seus handlers, e um
`Mediator<H>` ligado estaticamente (monomorfizado — um handler em falta é um erro
de compilação, não um panic em runtime). Middleware tipado embrulha o despacho:
`Instrument` (conta despachos) e `CacheQueries` (memoiza uma consulta, deixa passar
os comandos). Esta é a costura onde os atributos `#[cached]`/`#[policy]` da fachada
assentam.

## O servidor (`seiva-server`)

O runtime que um módulo instala — o "main":

- `serve(config, shutdown)` liga um listener, constrói a app a partir de
  `seiva_http::router` (controllers + `/openapi.json` + `/health`), comuta a saúde
  `Starting → Ready` uma vez ligado e `→ Draining` no sinal de encerramento, e
  serve com `axum::serve(..).with_graceful_shutdown(..)`.
- `run_workers(shutdown)` conduz passagens de fundo através do runtime de workers da
  framework; a passagem de relay do outbox DORM é injectada aqui por trás da feature
  `dorm`.
- `ServerConfig` é lido 12-factor (`seiva-config`): `SEIVA_HTTP_ADDR` > `PORT` >
  `127.0.0.1:8080`, mais `APP_ENV` e uma guarda que rejeita atalhos de dev em
  produção.

## Crates transversais

- `seiva-messaging` — um envelope CloudEvents e backends `Publisher` (NATS, AMQP,
  RabbitMQ; Kafka), cada um por trás de uma feature.
- `seiva-auth` — hashing de passwords Argon2id em formato PHC.
- `seiva-config` — configuração 12-factor tipada com a guarda de produção `AppEnv`.
- `seiva-webhooks` — assinatura/verificação Standard Webhooks (HMAC-SHA256, uma
  janela de replay, comparação em tempo constante).

## Fitness functions (`seiva lint`)

O módulo `lint` do `delonix-seiva` codifica a arquitectura como verificações: a
regra de dependência entre camadas e o contrato de zero-trust `[[uses]]`/`[[allow]]`
do módulo. Correm na CLI e são testadas unitariamente no CI, por isso um desvio das
regras falha o build em vez de ser entregue.

## DORM: por omissão, opcional e isolado

O DORM é a camada de dados por omissão mas entra através de exactamente um crate,
`seiva-data-dorm` (o padrão Gateway), por trás da feature `dorm`. Desligar a camada
de dados é simplesmente não compilar esse crate. Como o gateway depende do
repositório **privado** DORM por uma revisão git fixada, ele e os seus companheiros
(`seiva-outbox-relay`, `seiva-rag-worker`, `seiva-mcp`) são **excluídos do workspace
por omissão**: tudo o resto compila e testa sem credenciais.

Um job de CI gated por token (`dorm-gateway`) compila e testa esses crates contra o
repositório privado quando um segredo `DORM_TOKEN` está presente, e é um no-op verde
sem ele. A feature `dorm` fica por ligar no build por omissão de propósito: o job de
lint corre `cargo clippy --all-features`, por isso uma feature que puxasse o DORM
precisaria do token — ela activa-se assim que o segredo existir.

## Ver também

- [Quickstart](quickstart.md) — esqueletizar, correr e chamar um serviço
  ponta-a-ponta.
- [`docs/adr/`](adr/) — as decisões de desenho (fundações, a fronteira do ORM, a
  resolução do modelo de tx, …).

# Parte 1: O teu primeiro módulo — uma aplicação polls

Este tutorial constrói um pequeno serviço **polls** — a primeira aplicação
canónica. Podes fazer perguntas, dar a cada pergunta algumas opções, votar numa
opção e ler os resultados.

!!! info "Para onde este tutorial vai"
    Vais construir a polls primeiro como um **monólito modular** — um binário, um
    deploy — ao longo das Partes 1–6. Depois, na
    [Parte 7](part-7-monolith-to-microservice.md), vais promover o módulo `polls`
    ao seu **próprio microserviço** com um único comando, *sem alterar o código do
    módulo*. Esse arco — começar simples, separar quando precisas — é o ponto da
    fronteira de módulo do Seiva.

O Seiva é **API-first e compilado**: os teus handlers são simples `async fn`, um
módulo expõe **um contrato legível por máquina**, e as rotas HTTP, a documentação
interactiva da API e o serviço gRPC são todos *derivados* dele. O código terminado
deste tutorial é o módulo
[`examples/polls`](https://github.com/ngolacloud/seiva/tree/main/examples/polls)
no repositório.

Nesta parte geras o projecto e o módulo `polls`, e conheces as cinco camadas. (As
partes seguintes acrescentam validação, a base de dados, autenticação, testes e
empacotamento.)

## Criar o projecto

Começa por gerar o projecto. A CLI `seiva` cria um workspace Cargo — um monólito
modular — com um composition root, um kernel partilhado, um `schema/` e o
empacotamento cloud-native:

```console
$ seiva project new polls-site
$ cd polls-site
```

Compila de forma autónoma. A partir daqui podes comandar o build com o Cargo ou com
[**`dx`**](https://ngolacloud.github.io/dx/) (o gestor de pacotes da Delonix
compatível com o Cargo); os separadores abaixo mostram ambos — escolhe um.

=== "dx"

    ```console
    $ dx build          # compile the workspace
    ```

=== "cargo"

    ```console
    $ cargo build       # compile the workspace
    ```

## Adicionar o módulo polls

Adiciona agora o **módulo** `polls`:

```console
$ seiva module new polls
```

Isto gera `modules/polls/` como **cinco crates**, cada um uma camada, e liga
automaticamente o módulo ao composition root (o seu router é fundido e as suas deps
adicionadas):

- **`facade`** — o contrato versionado: um `#[seiva::facade] trait Polls` de
  `async fn`s, mais os DTOs de wire (`CreateQuestion`, `QuestionDto`, `ChoiceDto`, …)
  com `serde`, `Validate` e `WireSchema`. É aqui que vivem as formas de dados do
  módulo *e* as assinaturas dos seus métodos.
- **`domain`** — as entidades (`Question`, `Choice`) e as suas regras (o texto de
  uma pergunta e de uma opção não pode estar vazio). Não depende de nada além do
  kernel partilhado.
- **`application`** — os casos de uso que implementam a fachada sobre *portas*
  (`PollRepository`, `IdGenerator`, `UnitOfWork`, `EventPublisher`). Estes são os
  handlers de pedido: `list` devolve as perguntas mais recentes, `detail`/`results`
  devolvem uma pergunta com as suas opções, `vote` incrementa a contagem de uma
  opção. Livre de transporte e de base de dados.
- **`infrastructure`** — adaptadores: um `PollRepository` em memória por omissão
  (um suportado por DORM chega na Parte 3).
- **`interface`** — o `#[seiva::controller]` que transforma a fachada em rotas HTTP,
  mais o OpenAPI do módulo.

O compilador impõe a direcção das dependências, por isso a arquitectura não pode
apodrecer (ver [Arquitectura](../../architecture.md)).

## A estrutura de pastas do projecto

Depois de gerar o projecto e adicionar o módulo, o teu workspace fica assim:

```text
polls-site/
├── Cargo.toml                   # o workspace cargo
├── seiva.toml                   # configuração do projecto
├── apps/
│   └── polls-site-api/          # a raiz de composição (o binário)
│       └── src/main.rs
├── shared/
│   └── kernel/                  # o kernel partilhado (ids, erros, tempo)
├── modules/
│   └── polls/                   # o teu módulo — cinco camadas, um crate cada
│       ├── facade/              #   o contrato versionado + DTOs de fio
│       ├── domain/              #   entidades (Question, Choice) e regras
│       ├── application/         #   casos de uso sobre portas (sem transporte/BD)
│       ├── infrastructure/      #   adaptadores (em memória agora, DORM na Parte 3)
│       └── interface/           #   o controller + o OpenAPI do módulo
├── schema/
│   └── polls-site.schema        # o schema DORM (Parte 3)
├── Delonixfile                  # a imagem distroless (Parte 6)
└── compose.yaml                 # execução local (Parte 6)
```

Tudo o que está em `modules/` é teu; `apps/polls-site-api` é a raiz fina que
compõe os módulos num só binário. Adicionar outro módulo com
`seiva module new <nome>` cria outro `modules/<nome>/` e liga-o — o projecto
cresce por *adição* de pastas, nunca por edição de um registo central.

## O contrato que estás a construir

No fim do tutorial, a fachada `polls` lê-se assim (o ficheiro completo é
[`examples/polls/facade`](https://github.com/ngolacloud/seiva/tree/main/examples/polls/facade)).
Repara que cada método é `async fn` — os handlers são assíncronos por omissão:

```rust
#[facade(version = "v1")]
pub trait Polls {
    #[requires("polls.manage")]
    async fn ask(&self, cmd: CreateQuestion) -> Result<QuestionDto, String>;

    #[requires("polls.manage")]
    async fn add_choice(&self, cmd: AddChoice) -> Result<ChoiceDto, String>;

    #[requires("polls.read")]
    async fn list(&self) -> Result<Vec<QuestionDto>, String>;          // the index view

    #[requires("polls.read")]
    async fn detail(&self, q: GetQuestion) -> Result<QuestionDetail, String>;

    #[requires("polls.vote")]
    async fn vote(&self, cmd: Vote) -> Result<ChoiceDto, String>;      // the vote() view

    #[requires("polls.read")]
    async fn results(&self, q: GetQuestion) -> Result<QuestionDetail, String>;
}
```

Cada método nomeia a permissão que um chamador tem de possuir — o transporte
impõe-na (Parte 4). Não escreveste código de routing; as rotas vêm deste trait.

## Corre-o

=== "dx"

    ```console
    $ seiva start                 # dev loop (add --watch for hot-reload)
    # or directly: dx run -p polls-site-api
    ```

=== "cargo"

    ```console
    $ seiva start                 # dev loop (add --watch for hot-reload)
    # or directly: cargo run -p polls-site-api
    ```

O serviço serve `/health`, `/openapi.json`, a documentação interactiva
(`/docs` = Swagger UI, `/redoc` = ReDoc, `/scalar` = Scalar), mais as rotas do
módulo. Abre <http://localhost:8080/docs> e todo o contrato `polls` já está
documentado — não escreveste nenhuma spec. Confirma-o também a partir da shell:

```console
$ seiva openapi | grep -o '"/v1/polls"'
"/v1/polls"
```

Essa rota, o seu schema de pedido/resposta e as suas permissões `polls.*` foram
todos derivados da fachada — sem funções de vista, sem URLconf, sem serializadores
escritos à mão.

## A seguir

[Parte 2: Validação e DTOs](part-2-validation-and-dtos.md) — torna os corpos da
pergunta e da opção validados e honestos.

---

*Parte da [documentação do Seiva](../../index.md).*

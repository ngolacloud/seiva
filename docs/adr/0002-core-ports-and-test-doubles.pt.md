# ADR 0002 — Portas do núcleo e duplos de teste em memória

- Estado: Aceite (a forma do `UnitOfWork` emendada pela ADR 0004)
- Data: 2026-11-03
- Decisores: NgolaCloud
- Segue: ADR 0001

> **Emenda (ADR 0004).** Esta ADR descreveu primeiro o `UnitOfWork` como
> baseado em handle (`begin()` → `commit()`/`rollback()` + `on_commit`). O
> gateway do DORM revelou que o executor do DORM é delimitado por closure
> (`db::atomic`), que um commit baseado em handle não consegue conduzir, pelo
> que o `UnitOfWork` passa agora a ser **baseado em âmbito**:
> `transaction(|| async { … })` faz commit em `Ok` e rollback em `Err`, e a
> necessidade de "publicar após o commit" é satisfeita agindo sobre o `Ok`
> devolvido (sem hook `on_commit`). As outras três portas ficam inalteradas.

## Contexto

A camada de aplicação de cada módulo tem de ser escrevível, e testável, sem um
servidor, sem um broker ou uma base de dados, e sem depender da crate do
framework ou do DORM. Isso só é possível se as capacidades de que um caso de uso
precisa forem nomeadas como traits (portas) pequenas e estáveis, contra as quais
o caso de uso é codificado, com implementações concretas fornecidas no momento
da composição e falsos fornecidos sob teste.

Esta ADR fixa o conjunto de portas do núcleo, o modelo de erro na fronteira,
como as dependências são resolvidas, e onde vivem os duplos em memória. É o chão
sobre o qual cada fase posterior (camada de dados, runtime, scaffolding)
constrói, por isso as formas aqui são escolhidas para serem estáveis:
`seiva-ports` é estrito em SemVer por política.

## Decisão

### 1. As portas do núcleo

`seiva-ports` contém exactamente quatro traits, as que todo o módulo usa
independentemente da sua camada de dados:

- **`Clock`** — lê o tempo actual (`now() -> SystemTime`). Injectado para que os
  casos de uso sejam determinísticos: um falso devolve um instante fixo.
- **`IdGenerator`** — cunha identificadores (`new_id() -> Id`). Injectado para
  que um duplo de teste entregue uma sequência previsível. `Id` é um `[u8; 16]`
  sem dependências; o framework e os adaptadores renderizam-no como UUID/ULID.
- **`EventPublisher`** — publica um evento de domínio (`publish(topic,
  payload)`). O payload já vem serializado pelo chamador; se a publicação é
  transaccional (outbox) ou de melhor-esforço é uma propriedade da implementação
  e do `seiva.toml`, não do trait.
- **`UnitOfWork`** / **`UnitOfWorkFactory`** — a fronteira da transacção. A
  camada de aplicação decide onde uma transacção começa e termina, uma por caso
  de uso, através de `begin()` → `commit()`/`rollback()`. `on_commit` regista um
  callback que corre só após um commit bem-sucedido (p. ex. para acordar o relay
  do outbox), e é descartado no rollback.

As portas da camada de dados (`Connection` health/shutdown, `OutboxStore`,
`JobStore`) ficam adiadas para a fase da camada de dados e são acrescentadas
então; mantê-las de fora agora conserva esta fronteira mínima até haver uma
implementação real para as moldar.

### 2. Modelo de erro

As portas devolvem `PortResult<T> = Result<T, BoxError>`, onde `BoxError` é um
`std::error::Error + Send + Sync` em caixa. A fronteira não carrega nenhuma
biblioteca de erros, por isso não impõe nenhuma ao código a jusante; os
adaptadores mapeiam os seus erros concretos (um erro de BD, um erro de broker)
em `BoxError` no limite, e a camada de aplicação vê um único tipo de falha
uniforme. Os erros de domínio são uma preocupação separada, propriedade das
crates `domain`/`application`, e nunca atravessam esta fronteira como strings.

### 3. Dependências resolvidas em tempo de compilação

Um caso de uso declara as portas de que precisa na sua assinatura (genéricos ou
`dyn Port`). A composição liga as implementações concretas; não há reflexão em
runtime nem service locator. Um fornecedor em falta é um erro de build, não um
panic em runtime, e um ciclo entre módulos é também um erro de build. Sob teste,
um duplo é substituído directamente (o `app.override::<dyn Port>(fake)` do
brief), o que funciona precisamente porque o caso de uso depende do trait, nunca
da implementação.

### 4. Os duplos em memória vivem em `seiva-testing`

Para cada porta, o Seiva fornece um duplo de referência escrito à mão numa crate
dedicada `seiva-testing` que depende apenas de `seiva-ports` (mais
`async-trait`). Os módulos puxam-no como `dev-dependency`, por isso os duplos
nunca entram num build de release e a dependência de runtime da camada de
aplicação mantém-se só `seiva-ports`.

Os duplos **não** são colocados em `seiva-ports`: essa crate é só-traits, sem
lógica, e tem de assim ficar. Quando a fase de scaffolding gera duplos por
módulo (o "o Seiva gera um duplo em memória" do brief), estes duplos de
referência são a forma que ela emite e o recurso a que um módulo pode recorrer
directamente.

Os duplos fundacionais:

| Porta | Duplo | Comportamento |
| --- | --- | --- |
| `Clock` | `FixedClock` | Fixo num instante escolhido; `advance(Duration)` para asserções baseadas no tempo |
| `IdGenerator` | `SequenceIdGenerator` | Ids determinísticos a partir de um contador (big-endian para `Id`) |
| `EventPublisher` | `RecordingEventPublisher` | Regista cada `(topic, payload)` para asserções |
| `UnitOfWorkFactory` | `InMemoryUnitOfWorkFactory` | Regista o desfecho de cada transacção; o `UnitOfWork` corre os hooks `on_commit` no commit e descarta-os no rollback |

### 5. Política de estabilidade de `seiva-ports`

`seiva-ports` é estrito em SemVer: uma alteração que quebre uma porta é uma
versão maior. Mantém-se só-traits — sem runtime, sem macros — com `async-trait`
como sua única dependência (necessária para métodos async object-safe através da
fronteira `dyn Port`). É isto que permite a cada módulo e a cada duplo gerado ser
codificado contra uma fronteira que não oscila.

## Consequências

- Um teste unitário da camada de aplicação não precisa de `tokio` para além de
  um executor, nem de BD nem de broker: construa-se o handler com os quatro
  duplos e façam-se asserções sobre os eventos registados e os desfechos das
  transacções.
- A semântica `on_commit`-só-no-commit é observável e testada, por isso o
  posterior despertar do relay do outbox é construído sobre um contrato provado.
- Acrescentar uma porta da camada de dados mais tarde é aditivo (novo trait),
  não uma alteração que quebre as quatro fixadas aqui.

## Alternativas consideradas

- **Duplos por trás de uma feature `testing` de `seiva-ports`.** Rejeitada:
  colocaria lógica (mutabilidade interior, registo) na crate só-traits,
  quebrando o seu invariante "apenas traits" ainda que as dependências ficassem
  só-std.
- **Um service locator / contentor de DI em runtime.** Rejeitada: move
  "dependência em falta" de tempo de compilação para runtime, o que um framework
  de alta criticidade não pode fazer. A DI é resolvida pelo sistema de tipos.
- **Um enum de erro concreto nas portas.** Rejeitada: forçaria uma taxonomia de
  erros sobre a fronteira e deixaria escapar detalhe de implementação (qual
  backend falhou) para a camada de aplicação. `BoxError` mantém a fronteira
  neutra.

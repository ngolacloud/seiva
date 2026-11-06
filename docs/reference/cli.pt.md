# A CLI `seiva`

O `seiva` é a ferramenta de linha de comandos instalável (binário `seiva`, crate
`seiva-cli`): faz scaffold de projectos e módulos, corre o ciclo de dev, conduz a
camada de dados, faz lint à arquitectura, e imprime os contratos gerados. É a CLI
única do Seiva para todo o ciclo de vida do projecto.

Esta página é um **tour por ordem de ciclo de vida** — lê de cima a baixo e
percorres um projecto do nada até um serviço implantado e dividido. Cada passo
mostra a shell. Para o aprofundamento da camada de dados ver
[a camada de dados](../topics/data-layer/index.md); para as definições ver
[Definições e ambiente](settings.md).

## Instalar

A CLI é uma ferramenta global — instala-a com o Cargo (até os crates serem
publicados, a partir do workspace):

```console
$ cargo install --path crates/seiva-cli       # or, once published: cargo install seiva-cli
$ seiva --help
```

> **Utilizadores do `dx`:** para fixar o `seiva` como ferramenta local do projecto
> e offline, usa antes `dx tool install --path crates/seiva-cli` e corre-o via
> `dx tool run seiva …`. De resto o `dx` gere o build e as dependências do teu
> projecto (`dx build`, `dx run`, `dx add`), de forma compatível com o Cargo.

## Os comandos num relance

```
project  Create a new Seiva project
module   Manage modules within a project (new / split / merge)
ext      Integrate an optional extension (e.g. the admin console)
db       Data-layer commands (DORM modules only)
start    Run the project locally (the dev loop)
openapi  Print the OpenAPI 3.1 document for the project (or the demo facade)
proto    Print the proto3 service document
lint     Run the architecture fitness functions locally
expand   Show what the Seiva macros expand to
doctor   Report the environment: framework version and linked data layer
user     Manage user credential records (Argon2id)
```

## 1. Criar um projecto — `seiva project new <name>`

Faz scaffold de um workspace Cargo (um **monólito modular**): a raiz de composição
(`apps/<name>-api`), o kernel partilhado, um `schema/<name>.schema`, e o
empacotamento cloud-native (`Delonixfile`, `compose.yaml`, `.dockerignore`).
Compila de forma autónoma — a raiz do workspace carrega um `[patch.crates-io]`
provisório que aponta os crates do framework para um checkout local até serem
publicados (sobrepõe com `SEIVA_FRAMEWORK_DIR`; ver [Publicação](../publishing.md)).

```console
$ seiva project new polls-site
Created project `polls-site`. Next: cd polls-site && seiva module new <name>
$ cd polls-site
```

## 2. Verificar o ambiente — `seiva doctor`

Reporta a versão do framework e qual a camada de dados ligada (em-memória vs
DORM). Corre-o sempre que estejas inseguro sobre o que um binário fará.

```console
$ seiva doctor
```

## 3. Adicionar um módulo — `seiva module new <name> [--data external] [--admin]`

Faz scaffold de um módulo de cinco camadas (facade, domain, application,
infrastructure, interface) ligado aos transportes reais do framework, e regista-o
no workspace, no `seiva.toml` e no esquema — e funde o seu router na raiz de
composição, para que seja servido de imediato.

```console
$ seiva module new polls
Created module `polls` (facade v1, 5 layers). Registered in the workspace, seiva.toml and schema, and wired into the composition root.
```

`--admin` regista um recurso de administração (exposto pela extensão opcional
`seiva-console-admin`); `--data external` retira o módulo do DORM.

## 4. Corrê-lo — `seiva start`

Corre o servidor `apps/<name>-api` do projecto em desenvolvimento — o comando de
dev-server do Seiva. Adiciona `--watch` para hot-reload (precisa de `cargo-watch`).

```console
$ seiva start                 # or, directly: dx run -p polls-site-api / cargo run -p polls-site-api
```

O serviço serve `/health`, `/openapi.json`, e a documentação interactiva:
`/docs` (Swagger UI), `/redoc` (ReDoc) e `/scalar` (Scalar) — todas as três
geradas a partir das tuas fachadas sem anotações. Abre
<http://localhost:8080/docs> para experimentar chamadas contra o serviço em
execução.

## 5. Inspeccionar o contrato — `seiva openapi` / `seiva proto`

```console
$ seiva openapi               # the PROJECT's OpenAPI 3.1 (build+run, emit, exit); --compact for one line
$ seiva openapi | grep -o '"/v1/polls"'
"/v1/polls"
$ seiva proto                 # the project's proto3 service document
```

Dentro de um projecto estes documentam os **teus** módulos via descoberta de
fachadas em tempo de link (sem lista central — OWASP API9); corre-os fora de um
projecto e documentam antes a fachada de demonstração embutida do framework. Os
corpos são *fiéis* — o DTO directamente, com os seus campos reais (ver
[Documentos de API](../topics/api-documents.md)).

## 6. A camada de dados — `seiva db …` (módulos DORM)

```console
$ seiva db init                       # PostgreSQL (default); or --backend mysql | sqlite
$ export DELONIX_ORM_DSN="host=localhost user=me dbname=polls sslmode=disable"
$ seiva db dev initial                # makemigrations + migrate (author + apply the next migration)
$ seiva db migrate                    # migrate deploy (apply the recorded chain, in dependency order)
$ seiva db pull                       # brownfield: introspect an existing DB into a schema
$ seiva db studio                     # browse/edit data in the browser (Prisma Studio-style)
```

Estes delegam na CLI do DORM (`dorm`), encontrada no `PATH` ou via
`SEIVA_DORM_BIN`. Ver [Migrações](../topics/data-layer/migrations.md) e
[Fazer consultas](../topics/data-layer/queries.md).

## 7. Dividir um módulo num microserviço — `seiva module split` / `merge`

Promove um módulo ao seu próprio serviço sem mudar o seu código: `split` faz
scaffold de um binário autónomo `apps/<name>-service` que monta apenas esse
módulo, regista-o no workspace, e define `mode = "service"` no `module.toml` do
módulo. O módulo continua a correr dentro do monólito também — isto acrescenta uma
forma de deploy.

```console
$ seiva module split polls
Split module `polls` into a service. Created apps/polls-service (mounts only `polls`), registered it in the workspace and set mode = "service".
$ cargo run -p polls-service          # or dx run -p polls-service — serves ONLY polls
$ seiva module merge polls            # fold it back: removes the service, resets mode = "library"
```

Ver o [monólito → microserviço](../getting-started/shop/part-7-monolith-to-microservices.md)
do tutorial.

## 8. Adicionar uma extensão opcional — `seiva ext add <name>`

Integra uma extensão opcional na raiz de composição (ADR 0010): adiciona a
dependência e insere a montagem na âncora gerida — explícita, idempotente, e não
executa nenhum script de instalação.

```console
$ seiva ext add seiva-console-admin   # mount the admin console at /admin
```

## 9. Manter a arquitectura honesta — `seiva lint` / `seiva expand`

```console
$ seiva lint                  # the dependency rule + the anti-pattern catalogue
$ seiva expand                # show what #[facade]/#[controller]/#[derive(WireSchema)] expand to
```

`lint` é a passagem de funções de fitness (ver [Testes](../topics/testing.md));
`expand` é o inspector "sem magia escondida".

## 10. Utilizadores — `seiva user create --username <NAME>`

```console
$ seiva user create --username alice  # an Argon2id credential record; the password is read from STDIN
```

## Relacionado

- [Primeiros passos](../getting-started/index.md) · [Arranque rápido](../quickstart.md)
- [Definições e ambiente](settings.md)
- [Os crates](crates/index.md)

---

*Parte da [documentação do Seiva](../index.md).*

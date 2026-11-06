# Usar o dx com o Seiva

Um projecto Seiva é um workspace Cargo vulgar, por isso podes conduzi-lo com ou o
`dx` ou o `cargo`. Este guia mostra o ciclo de ponta a ponta com o `dx`, explica
como o `dx` se relaciona com a CLI `seiva`, e documenta a convenção de separadores
`dx`/`cargo` que vês por todos os tutoriais.

!!! info "Duas ferramentas, dois trabalhos"
    - **O `seiva`** é a CLI do próprio framework — faz o scaffold de projectos e
      módulos, gera os contratos OpenAPI/gRPC, corre as verificações de fitness,
      e conduz o **ciclo de dev** (`seiva start`).
    - **O `dx`** é o **gestor de pacotes e executor** subjacente — adiciona
      dependências (com um plano e uma barreira), constrói, corre, testa, audita,
      e empacota.

    São complementares: o `seiva` é dono do fluxo de desenvolvimento; o `dx` (ou
    o `cargo`) é dono do ciclo de vida de build e de dependências por baixo dele.
    Nenhum requer o outro.

## A convenção de separadores

Cada comando de build/run nos tutoriais aparece em dois separadores — escolhe a
ferramenta que usas. Os comandos são equivalentes; o manifesto sobre o qual operam
é o mesmo workspace Cargo simples.

=== "dx"

    ```console
    $ dx build
    ```

=== "cargo"

    ```console
    $ cargo build
    ```

## Conduzir um projecto Seiva de ponta a ponta

Parte de um projecto feito com scaffold (vê o [Arranque rápido](../quickstart.md)
para `seiva project new`). A partir da raiz do projecto:

### Construir o workspace

=== "dx"

    ```console
    $ dx build            # compile the whole workspace
    $ dx build --release  # optimized build
    ```

=== "cargo"

    ```console
    $ cargo build
    $ cargo build --release
    ```

### Correr o binário da API

Um projecto Seiva expõe o seu serviço HTTP como a crate `<project>-api`. Para um
projecto chamado `polls-site`, essa crate é `polls-site-api`:

=== "dx"

    ```console
    $ seiva start                      # the dev loop (add --watch for hot-reload)
    # or run the API crate directly:
    $ dx run -p polls-site-api
    ```

=== "cargo"

    ```console
    $ seiva start                      # the dev loop (add --watch for hot-reload)
    # or run the API crate directly:
    $ cargo run -p polls-site-api
    ```

`seiva start` é o ciclo de dev recomendado — é o trabalho da CLI `seiva`. Quando
queres correr exactamente uma crate, ou ser explícito no CI, `dx run -p
<project>-api` (o selector de pacote `-p` do Cargo passa directamente) fá-lo. O
serviço serve `/health`, `/openapi.json` e os docs interactivos, exactamente como
descrito no tutorial.

### Testar

=== "dx"

    ```console
    $ dx test --suite unit
    $ dx test --suite integration --format junit --out target/report.xml
    ```

=== "cargo"

    ```console
    $ cargo test
    ```

`dx test --suite` acrescenta suites nomeadas e formatos de relatório
(JUnit/JSON/HTML) sobre o executor de testes do Cargo — conveniente para ligar a
saída de testes do Seiva ao CI.

### Manter a arquitectura honesta

As funções de fitness permanecem uma responsabilidade do `seiva` — são a camada do
Seiva e verificações de zero-trust, não um conceito do Cargo:

```console
$ seiva lint
```

Corre `dx audit` ao lado dele para obteres um veredicto de segurança sobre o
`Cargo.lock` resolvido (vê a nota sobre a barreira abaixo).

## Adicionar dependências num workspace Seiva

Usa `dx add` a partir da raiz do projecto, como de costume. Pré-visualiza
primeiro, depois deixa-a aterrar:

```console
$ dx add time@0.3 --plan     # show the impact before anything is written
$ dx add time@0.3            # land it (idempotent)
```

!!! warning "A barreira é 'não aplicável' num workspace"
    Um projecto Seiva é um **workspace** Cargo, e a barreira de segurança do
    `dx add` reporta **não aplicável** para grafos em formato de workspace e de
    path: a dependência *aterra*, mas é marcada **não verificada** (saída `6`).
    Para obteres um veredicto de segurança real, corre `dx audit` sobre o lockfile
    resolvido depois de o build o resolver:

    ```console
    $ dx build       # resolves Cargo.lock
    $ dx audit       # verdict over the project's Cargo.lock
    ```

    Ligar `dx audit` ao CI é a forma fiável de impor a barreira para um workspace
    Seiva.

## O adaptador de templates do Seiva

O `dx` entrega um adaptador Seiva para poder fazer o scaffold e estender uma
plataforma Seiva com **composição gerida pelo dx** — uma integração declarativa,
baseada em âncoras, que nunca corre os hooks de build de uma crate.

### Fazer o scaffold de uma plataforma Seiva gerida pelo dx

```console
$ dx new my-platform --template seiva
```

Isto faz o scaffold de uma plataforma Seiva cuja composição é gerida pelo `dx`: um
`src/composition.rs` que carrega âncoras `// dx:seiva:*`, mais um marcador
`[package.metadata.dx.seiva].composition`.

### Adicionar uma extensão Seiva

Quando a composição é gerida pelo dx, `dx add` consegue ligar uma extensão por
âncoras — idempotentemente e reversivelmente:

```console
$ dx add seiva-console-admin
```

O `dx` adiciona a dependência através da barreira e, se a composição for gerida,
integra a extensão por âncoras (idempotente, reversível) — **nunca** correndo os
hooks da crate, não exigindo permissões de administração, e não definindo
palavras-passe. Depois recompilas com `dx build` para a activar.

!!! note "A composição não gerida produz um estado parcial — honestamente"
    Se a composição **não** for gerida pelo dx, `dx add seiva-console-admin` ainda
    adiciona a dependência mas deixa um **patch pendente** e reporta um estado
    parcial (saída `6`) em vez de reclamar uma integração que não realizou. Depois
    ligas a extensão tu próprio.

## Qual ferramenta, quando, num projecto Seiva

- Recorre ao **`seiva`** para tudo em formato de fluxo de trabalho:
  `seiva project new`, `seiva module new`, `seiva start`, `seiva openapi`/`seiva
  proto`, `seiva lint`.
- Recorre ao **`dx`** (ou ao `cargo`) para o ciclo de vida de build/dependências:
  construir, correr uma crate específica, adicionar/remover dependências com um
  plano, auditar, testar com relatórios, e empacotar.
- Prefere **`dx add --plan`** a um add às cegas quando trazes uma nova crate, e
  corre **`dx audit`** (em especial no CI) porque a barreira é apenas consultiva
  num workspace no momento do `add`.

Para a superfície completa de comandos `dx`, vê [Gestão de pacotes](package-management.md);
para as diferenças precisas em relação ao Cargo, vê [dx vs Cargo](dx-vs-cargo.md).

---

*Parte da [documentação do Seiva](../index.md).*

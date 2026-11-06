# Gestão de pacotes com o dx

Esta página é uma referência aos subcomandos `dx` que usas para gerir um projecto
no dia a dia: scaffolding, adicionar e remover dependências, construir, correr,
testar, e verificar o grafo de dependências. Cada comando aqui é respaldado pela
CLI `dx` real; onde um comando delega no Cargo, isso é assinalado.

!!! note "Códigos de saída"
    O `dx` usa códigos de saída com significado: `0` sucesso · `1` erro · `3` um
    comando planeado mas ainda não implementado · `4` a barreira de segurança
    **bloqueou** uma mudança · `5` a mudança está **pendente** (por exemplo, a
    base de dados de avisos está obsoleta) · `6` a barreira foi **não aplicável**
    e a mudança aterrou *não verificada* · caso contrário o código de saída da
    invocação Cargo subjacente. A saída estruturada vai para stdout; o plano e o
    progresso vão para stderr.

## Fazer o scaffold de um projecto

`dx new` cria um novo projecto; `dx init` faz o mesmo no directório actual. Ambos
se recusam a atropelar — `new` rejeita um directório não vazio e `init` recusa se
já existir um `Cargo.toml`. Ambos imprimem o plano primeiro, e `--dry-run` não
escreve nada.

```console
$ dx new my-service --bin        # or --lib
$ cd my-service
$ dx init --lib                  # same, but in the current directory
```

## Adicionar uma dependência

`dx add` adiciona ou actualiza uma dependência **idempotentemente**: as
funcionalidades são normalizadas (a ordem e os duplicados não importam), por isso
repetir o mesmo `add` é um no-op byte a byte.

```console
$ dx add serde@1 --features derive
$ dx add serde@1 --features derive          # no-op: nothing changes
$ dx add serde@1 --features derive --dry-run # print the plan, write nothing
```

O spec é `name` ou `name@<req>`. Opções comuns:

| Opção | Significado |
|--------|---------|
| `--features a,b` | activa funcionalidades (normalizadas) |
| `--no-default-features` | larga as funcionalidades por omissão |
| `--optional` | dependência opcional |
| `--dev` / `--build` | visa a secção `dev-` ou `build-dependencies` |
| `--git <url>` com `--rev` / `--tag` / `--branch` | uma fonte git |
| `--path <dir>` | uma fonte de path local |
| `--registry <name>` | um registo alternativo (versão obrigatória) |
| `--manifest-path <Cargo.toml>` | opera sobre um manifesto específico |

`dx add` impõe algumas regras honestas:

- Mudar a **fonte** de uma dependência existente é um erro — remove-a primeiro,
  depois adiciona-a de novo. O `dx` nunca troca uma fonte em silêncio.
- Uma dependência `--registry` **sem** versão é rejeitada; usa `name@<req>`.
- Um nome que pareça um typosquat de uma crate popular é recusado, com uma
  sugestão.

### A barreira de segurança no `add`

Antes de a dependência aterrar, o `dx` resolve o grafo candidato numa área de
staging isolada (`cargo generate-lockfile`, **sem** correr qualquer `build.rs`) e
corre a sua barreira de segurança:

- **Aprovado** → a dependência aterra.
- **Bloqueado** (saída `4`) → **não** aterra; o manifesto fica intocado.
- **Pendente** / base de dados de avisos obsoleta (saída `5`) → **não** aterra.
- **Não aplicável** — para grafos em formato de workspace ou de path — a
  dependência aterra mas é marcada *não verificada* (saída `6`); corre
  [`dx audit`](#auditar-o-grafo) depois.

Usa `--offline` para resolver só a partir da cache local. A idade máxima tolerada
da base de dados de avisos é configurável via `DX_MAX_DB_AGE_SECS`.

!!! info "Porque o `--plan` vale a pena"
    `dx add --plan` mostra o impacto completo *antes* de qualquer coisa aterrar: o
    grafo que a adição introduz, se puxa `build.rs` / proc-macros / código nativo,
    o seu efeito no teu MSRV, o veredicto de segurança, e se é servido da cache ou
    obtido de novo. É a contraparte explicável de um `cargo add` às cegas.

## Remover uma dependência

`dx remove` remove uma dependência e é idempotente — remover algo que não está
presente é um no-op.

```console
$ dx remove serde
$ dx remove some-dev-tool --dev
```

Aceita `--dev` / `--build`, `--manifest-path`, e `--dry-run`.

## Build, check, run, format, lint

Estes delegam no Cargo com um vector de argumentos estruturado (sem shell),
herdando o seu stdio e código de saída:

```console
$ dx check
$ dx build [--release]
$ dx run [-- <args>]
$ dx fmt [--check]
$ dx lint
```

!!! warning "Os comandos de passagem não correm a barreira"
    Só `dx add` aplica a barreira de segurança. `dx build`, `check`, `run`, `fmt`
    e `lint` são passagens para o Cargo e **não** voltam a correr a barreira.
    Corre [`dx audit`](#auditar-o-grafo) (ou liga-o ao CI) para verificar um
    `Cargo.lock` existente.

### Perfis de build e medição

`dx build --profile` aplica perfis LLVM através de `cargo --config` — sem tocar no
teu `Cargo.toml`:

```console
$ dx build --profile release       # ThinLTO + 16 codegen units (balanced default)
$ dx build --profile throughput    # fat-LTO + 1 codegen unit (opt-in, slow)
$ dx build --profile size          # strips the binary
$ dx build --profile release --native --measure
```

`dev`, `release`, `throughput` e `size` são os perfis disponíveis. Nenhum perfil
altera alguma vez as definições de panic, overflow ou segurança. `--native`
adiciona `target-cpu=native` (com um aviso — não para releases distribuídas).
`--measure` reporta tempo de build, pico de memória e tamanho do binário.

`dx bench-build [--profiles a,b,c]` faz um build **limpo** por perfil e tabula o
tempo de relógio de parede, o pico de memória e o tamanho do binário para poderes
comparar perfis honestamente.

### Optimização guiada por perfil

`dx pgo` corre PGO real: instrumenta o build, corre uma carga de trabalho (o
próprio binário ou um comando que dás), funde com `llvm-profdata`, recompila com o
perfil, e compara.

```console
$ dx pgo --bin my-service -- --some-workload-args
```

!!! note
    `dx pgo` prefere o `llvm-profdata` do sysroot do Rust e reporta quando uma
    versão é incompatível. Um ganho de desempenho real ainda exige medir o serviço
    sob carga.

## Testar com suites e relatórios

`dx test --suite` corre uma suite nomeada e emite um relatório. As suites
embutidas são `unit`, `doc` e `integration` (as embutidas do Cargo); podes definir
mais em `dx.toml` sob `[test.suites.<name>]` com um `runner` mais um `kind` de
`rust`, `command` ou `load`. Um nome de suite sem configuração é rejeitado.

```console
$ dx test --suite unit
$ dx test --suite integration --format junit --out target/report.xml
$ dx test --suite smoke --format html --out target/report.html
```

`--format` aceita `terminal`, `json`, `junit` ou `html`; adiciona `--no-ansi` para
saída de terminal simples. A saída JSON é versionada, o HTML é auto-contido, e em
todos os formatos os segredos são depurados e o HTML/XML é escapado. Um erro de
runner nunca reporta verde. As suites `load` falham em limiares de
p95/p99/taxa-de-erro/rps, não só no código de saída (as métricas são lidas de
`DX_LOAD_METRICS` ou `target/dx-load-metrics.json`).

## Verificar o grafo de dependências

### Auditar o grafo

`dx audit` corre a barreira de segurança sobre o `Cargo.lock` do projecto e
reporta achados por categoria, o estado da base de dados de avisos, e uma decisão
(saída `0`/`4`/`5`). `dx doctor` diagnostica a tua toolchain, o `cargo-audit`, a
frescura da base de dados de avisos, e a política.

```console
$ dx audit
$ dx doctor
```

### Verificar a integridade sem compilar

`dx fetch` verifica a integridade do grafo bloqueado **sem compilar** — faz
checksum dos ficheiros `.crate` em cache contra o `Cargo.lock`:

```console
$ dx fetch --locked --extract-check
```

`--locked` exige um `Cargo.lock`. `--extract-check` desempacota cada crate para um
directório temporário com salvaguardas contra travessia de path, fuga de
symlink/hardlink e zip-bombs, depois descarta-o. Um descasamento de checksum ou
uma extracção insegura é **bloqueado** (saída `4`). Os pacotes que nunca foram
descarregados são reportados como "não cobertos".

### Explicar o grafo

`dx why` e `dx explain` respondem *porquê* uma crate, funcionalidade ou duplicado
está no teu grafo (construído sobre `cargo tree`).

```console
$ dx why smallvec
```

## Actualizar dependências, explicavelmente

`dx update` torna as actualizações explicáveis e recuperáveis:

```console
$ dx update --plan                 # show version changes; lockfile untouched
$ dx update --security             # run the gate before/after; report what an
                                   # update fixes and what has no safe fix
$ dx update serde --apply --test   # write the new lockfile transactionally,
                                   # validate with the gate (and tests)
```

`--plan` resolve em staging e deixa o lockfile intacto. `--security` corre a
barreira antes e depois e reporta que avisos uma actualização corrige — e quais
não têm correcção compatível (explica o bloqueio em vez de trocar bibliotecas nas
tuas costas). `--apply` escreve o lockfile actualizado transacionalmente,
valida-o com a barreira (e com `--test` se dado), e **restaura** o lockfile
anterior se a validação falhar. Nomear um pacote (por exemplo `serde`) restringe a
mudança à actualização mínima desse único pacote.

## O lockfile e os workspaces

O `dx` não introduz um lockfile próprio — opera sobre o `Cargo.lock` padrão.
`dx fetch --locked` e `dx audit` lêem-no, e `dx update --apply` reescreve-o
transacionalmente.

Para **workspaces** (que é o que um projecto Seiva é), atenta no comportamento da
barreira: um grafo em formato de workspace ou de path faz a barreira do `dx add`
reportar **não aplicável** — a mudança aterra mas é marcada não verificada (saída
`6`) — por isso corre `dx audit` sobre o `Cargo.lock` resolvido para obteres um
veredicto. O `dx` toma um lock consultivo de workspace enquanto muta, para que
operações concorrentes não corrompam o manifesto.

## Fixar ferramentas de projecto e capturar o ambiente

`dx tool` fixa ferramentas de programador (por oposição a dependências de
biblioteca) por projecto:

```console
$ dx tool install cargo-nextest@0.9    # pinned in dx-tools.toml, into .dx/tools
$ dx tool run cargo-nextest -- run     # runs only the pinned tool (sha256-checked)
$ dx tool run cargo-nextest --isolated -- run
$ dx tool list
```

`install` regista a proveniência (fonte, versão, sha256) e instala com
`cargo install --locked` em `.dx/tools`. `run` executa **apenas** a ferramenta
fixada, verificando o seu sha256 (anti-adulteração); nunca recorre a um binário do
sistema, e `--isolated` corre-a sob o sandbox (rede desligada, ambiente despojado
de segredos).

!!! note "Ferramentas pré-compiladas verificadas estão planeadas"
    Por omissão `dx tool install` compila a versão fixada. Binários pré-compilados
    verificados (via `cargo-binstall` sob política) são um caminho futuro
    planeado.

`dx environment capture|verify` regista um ambiente de build suficientemente
reproduzível (toolchain, target, ferramentas auxiliares, flags, evidência de
segurança e o sha256 do `Cargo.lock`) num `dx-environment.json` para fazer commit
e partilhar, depois detecta **deriva** contra o host:

```console
$ dx environment capture
$ dx environment verify            # critical drift fails with exit 4
```

Isto é registo de ambiente mais detecção de deriva — não reprodutibilidade byte a
byte.

## Construir offline e entregar on-premises

`dx build --isolated` compila com isolamento **real**: a rede é desligada via um
namespace de utilizador/rede, o ambiente é limpo até uma allowlist mínima (sem
fuga de segredos), e limites de recursos são aplicados. Usa `--offline` e desactiva
qualquer wrapper de `rustc` que alcance a rede, e reporta exactamente o que foi
imposto.

```console
$ dx build --isolated
```

!!! warning "O isolamento do sistema de ficheiros ainda não é imposto"
    Nesta versão, `dx build --isolated` não impõe o isolamento do sistema de
    ficheiros — reporta isso como `false`. O isolamento de rede, o ambiente
    despojado e os limites de recursos *são* impostos.

`dx bundle` produz um pacote offline verificável para builds air-gapped ou
on-premises:

```console
$ dx bundle export --out ./offline
$ dx bundle verify ./offline       # recompute hashes; any mismatch fails (exit 4)
```

`export` escreve `Cargo.toml`/`Cargo.lock`, `environment.json`, `sbom.cdx.json`,
uma árvore `vendor/` vendorizada mais `cargo-config.toml` (via `cargo vendor`), e
um `bundle.json` com um sha256 por ficheiro e um hash agregado. `verify` recalcula
os hashes e falha em qualquer divergência. Um pacote verificável — de novo, não
reprodutibilidade byte a byte.

## Preparar uma release

`dx release prepare` valida num ambiente limpo (fmt / `clippy -D` / test / build
mais a barreira de segurança; e API/semver via `cargo-semver-checks` quando está
presente) e produz artefactos de revisão sob `target/dx-release/`: um SBOM
CycloneDX, proveniência, a ordem topológica de publicação, e binários com
checksums.

```console
$ dx release prepare
```

!!! important "Preparar não é publicar"
    `dx release prepare` não cria tags, não muda versões, nem publica. É
    idempotente — apenas reescreve `target/dx-release/` para revisão. Uma base de
    dados de avisos obsoleta fá-lo reportar **NOT READY**.

---

*Parte da [documentação do Seiva](../index.md).*

# dx vs Cargo

O `dx` **não** é um substituto do Cargo — é uma camada sobre ele. O Cargo e o
`rustc` continuam a resolver e a compilar; o `dx` conduz-nos e acrescenta
explicabilidade, segurança e recuperabilidade em torno do ciclo de vida das
dependências. Esta página compara os dois com precisão para que possas decidir a
qual recorrer.

## A versão de uma frase

Usa o `cargo` e obténs o resolvedor e o compilador do Rust. Usa o `dx` e obténs o
mesmo resolvedor e compilador, mais um plano antes de cada mudança de dependência,
uma barreira de segurança que corre antes de qualquer coisa ser construída, e
mutações transacionais de que podes recuperar. O `Cargo.toml` é idêntico em
qualquer dos casos.

## Equivalências de comandos

| Tarefa | `dx` | `cargo` |
|------|------|---------|
| Novo projecto | `dx new <name> [--lib\|--bin]` | `cargo new <name> [--lib\|--bin]` |
| Init no lugar | `dx init [--lib\|--bin]` | `cargo init [--lib\|--bin]` |
| Adicionar uma dependência | `dx add <spec> [--features …]` | `cargo add <spec> [--features …]` |
| Remover uma dependência | `dx remove <name>` | `cargo remove <name>` |
| Build | `dx build [--release]` | `cargo build [--release]` |
| Verificação de tipos | `dx check` | `cargo check` |
| Correr | `dx run [-- <args>]` | `cargo run [-- <args>]` |
| Formatar | `dx fmt [--check]` | `cargo fmt [--check]` |
| Lint | `dx lint` | `cargo clippy` |
| Testar | `dx test --suite <name>` | `cargo test` |
| Actualizar dependências | `dx update [name] [--plan\|--apply]` | `cargo update [name]` |
| Explicar o grafo | `dx why <crate>` | `cargo tree -i <crate>` |
| Auditar avisos | `dx audit` | `cargo audit` (externo) |

`dx build`, `check`, `run`, `fmt`, `lint` e o executor de testes delegam
directamente no Cargo com um vector de argumentos estruturado, herdando o seu
stdio e código de saída — por isso comportam-se exactamente como esperas do Cargo.

## O que o `dx` acrescenta sobre o Cargo

Estes **não têm equivalente directo no Cargo** — são a razão de o `dx` existir:

- **Um plano de impacto antes de uma mudança aterrar** — `dx add --plan` mostra o
  grafo introduzido, se traz `build.rs`/proc-macros/código nativo, os efeitos no
  MSRV, o veredicto de segurança, e cache-vs-novo, *antes* de escrever ao
  `Cargo.toml`. O `cargo add` apenas escreve.
- **Uma barreira de segurança ligada ao `add`** — `dx add` resolve o grafo
  candidato em staging isolado (**sem execução de `build.rs`**) e só faz aterrar a
  dependência se o veredicto for **Aprovado**; bloqueia (saída `4`) ou adia (saída
  `5`) caso contrário. O `cargo add` não tem conceito de segurança.
- **Mutações idempotentes e transacionais** — as funcionalidades são
  normalizadas, repetir um `add`/`remove` é um no-op, e cada escrita é jornalizada
  (`tmp + fsync + rename`) para que uma operação interrompida recupere limpamente.
- **Actualizações explicáveis e recuperáveis** — `dx update --security` reporta
  que avisos uma actualização corrige e quais não têm correcção compatível;
  `--apply` restaura o lockfile anterior se a validação falhar.
- **Uma barreira embutida e ferramentas de integridade** — `dx audit`/`dx doctor`
  e `dx fetch --extract-check` (checksum-vs-lockfile mais extracção segura contra
  travessia/symlink/zip-bomb) são parte da ferramenta, não uma instalação à parte.
- **Perfis de build, medição e PGO como comandos de primeira classe** —
  `dx build --profile …`, `dx bench-build`, `dx pgo`, aplicados via `cargo
  --config` sem editar o teu `Cargo.toml`.
- **Suites de teste configuráveis com relatórios** — `dx test --suite` com saída
  `terminal`/`json`/`junit`/`html`, depuração de segredos, e avaliação de limiares
  de carga.
- **Ferramentas fixadas ao projecto, captura de ambiente, pacotes offline,
  preparação de release** — `dx tool`, `dx environment`, `dx bundle`,
  `dx release prepare`.

## Onde o `dx` é compatível com o Cargo

- **O manifesto é Cargo simples.** O `dx` escreve entradas `Cargo.toml` vulgares.
  Um projecto feito com scaffold pelo `dx` compila e testa com o `cargo` e sem o
  `dx` instalado — um critério de aceitação deliberado.
- **O resolvedor e o compilador são o Cargo e o `rustc`.** O `dx` nunca os faz
  fork nem os reimplementa.
- **O lockfile é o `Cargo.lock`.** O `dx` não introduz lockfile próprio.
- **Os perfis são aplicados através de `cargo --config`,** não reescrevendo o teu
  manifesto.

Podes mover-te entre as duas ferramentas livremente no mesmo projecto, e
misturá-las — `dx add` uma dependência, depois `cargo build`, ou vice-versa.

## Limitações honestas

Atenta nestas ao escolher o `dx`:

- **Chamar o `cargo` directamente contorna a barreira.** Como o manifesto fica
  Cargo simples, `cargo add`/`cargo build` saltam a barreira de segurança do `dx`
  por completo. Se a barreira te importa, impõe o `dx` (ou a barreira) no CI e num
  ambiente de build controlado — o `dx` é opcional por desenho.
- **Os comandos de passagem não correm a barreira.** Só `dx add` a corre;
  `dx build`/`run`/`check`/`fmt`/`lint` são passagens puras. Usa `dx audit`
  (ou o CI) para verificar um `Cargo.lock` existente.
- **A barreira é "não aplicável" em workspaces e grafos de path.** Num workspace
  (que um projecto Seiva é), `dx add` faz aterrar a mudança mas marca-a
  *não verificada* (saída `6`); obténs um veredicto correndo `dx audit` sobre o
  `Cargo.lock` resolvido. Vê [Usar o dx com o Seiva](with-seiva.md).
- **O isolamento do sistema de ficheiros não é imposto** em `dx build --isolated`
  ainda (o isolamento de rede, um ambiente despojado e limites de recursos são).
- **Ferramentas pré-compiladas verificadas estão planeadas.** `dx tool install`
  compila a versão fixada hoje; binários verificados ao estilo `cargo-binstall`
  são um caminho futuro.

## Quando usar qual

| Usa o `dx` quando… | Usa o `cargo` quando… |
|----------------|-------------------|
| Queres ver o impacto de uma dependência antes de aterrar (`dx add --plan`). | Já sabes exactamente o que estás a adicionar e queres o caminho mais curto. |
| Queres que a barreira de segurança avalie as adições antes de compilarem. | Estás num ciclo interno apertado e a barreira é imposta noutro lado (CI). |
| Queres actualizações explicáveis e recuperáveis (`dx update --security/--apply`). | Só precisas de um `cargo update` rápido. |
| Estás a produzir um SBOM, um pacote offline, ou a preparar uma release. | Estás a confiar noutro pipeline de release. |
| Queres ferramentas de projecto fixadas e com proveniência rastreada. | Ferramentas instaladas no sistema servem para a tua configuração. |

Como as duas partilham o manifesto e o lockfile, isto não é um compromisso de
um-ou-outro: adopta o `dx` onde as suas garantias ajudam, e recorre ao `cargo`
para o resto.

---

*Parte da [documentação do Seiva](../index.md).*

# dx — o gestor de pacotes da Delonix

O `dx` é o gestor de pacotes e ferramenta de build da Delonix para Rust. É
**compatível com o Cargo por desenho**: o Cargo e o `rustc` continuam a resolver e
a compilar o teu código, por isso os manifestos que o `dx` escreve são manifestos
Cargo simples. Por cima disso, o `dx` responde à pergunta que o Cargo te deixa —
*"o que estou realmente a puxar, e é seguro?"* — tornando cada mudança de
dependência **explicável**, **verificada em segurança antes de aterrar** e
**recuperável**.

O binário é `dx`. É inspirado pela clareza do
[uv](https://docs.astral.sh/uv/) para Python, respeitando deliberadamente os
mecanismos nativos do Rust: o `dx` nunca substitui o resolvedor nem o compilador,
conduz-nos.

!!! info "Opcional, como o resto da Delonix"
    O `dx` não requer o Seiva, e o Seiva não requer o `dx`. Um projecto
    Seiva é um workspace Cargo vulgar — podes construí-lo e corrê-lo com
    **ou o `dx` ou o `cargo`**. Os tutoriais mostram ambos em separadores; escolhe
    um. O `dx` acrescenta valor quando queres mudanças de dependência explicáveis
    e auditáveis, mas nunca é obrigatório.

## Porque existe

O Cargo é excelente a resolver e a construir. O que não faz é dizer-te, *antes* de
uma dependência aterrar, o que essa dependência arrasta, se carrega um aviso
conhecido, ou se corre código de build nativo. O `dx` preenche exactamente essa
lacuna:

- **Mudanças explicáveis.** `dx add --plan` mostra o impacto de uma adição —
  o grafo que introduz, se traz `build.rs`/proc-macros/código nativo, o seu efeito
  no teu MSRV, e o veredicto de segurança — *antes* de escrever uma única linha ao
  `Cargo.toml`.
- **Segurança antes da execução.** `dx add` resolve o grafo candidato numa área de
  staging isolada (**sem** correr qualquer `build.rs`) e corre uma barreira de
  segurança. A dependência só aterra se o veredicto for **Aprovado**.
- **Recuperação previsível.** Cada mutação é transacional (jornal mais
  `tmp + fsync + rename`), por isso uma operação interrompida recupera limpamente
  na execução seguinte; repetir um comando é um no-op.

A fundamentação do desenho vive nas ADRs do repositório do `dx` — vê
[ADR-0015 (posicionamento e plano de impacto)](https://github.com/ngolacloud/dx/blob/main/docs/adr/0015-posicionamento-e-plano-de-impacto.md).

## Instalar

O `dx` instala-se com o Cargo, como qualquer outra ferramenta Rust. A crate
publicada é `ngoladx-cli` (os nomes curtos já estavam tomados no crates.io), e
fornece o binário `dx`:

```console
$ cargo install ngoladx-cli      # installs the `dx` binary
$ dx --help
```

Há também um instalador de uma linha que usa o Cargo por baixo:

```console
$ curl -fsSL https://ngolacloud.github.io/dx/install.sh | bash
```

!!! note "Os nomes de importação em Rust não mudam"
    Só os *nomes de crate publicados* carregam o prefixo `ngoladx-`. O binário
    `dx`, o teu `Cargo.toml`, e os caminhos de importação Rust ficam todos
    inalterados — o `dx` escreve manifestos Cargo simples.

## Um primeiro gosto

```console
$ dx new my-lib --lib
$ cd my-lib
$ dx add serde@1 --features derive          # idempotent — repeating is a no-op
$ dx add serde@1 --features derive --dry-run # show the plan, write nothing
$ dx build
```

O `Cargo.toml` gerado é Cargo puro: compila e testa **sem** o `dx` instalado. Esse
é um critério de aceitação deliberado — o `dx` é uma camada sobre o Cargo, não um
fork dele.

!!! warning "Chamar o `cargo` directamente contorna a barreira"
    Como o manifesto fica Cargo simples, invocar o `cargo` directamente salta a
    barreira de segurança do `dx` — por desenho. Se a tua organização confia na
    barreira, impõe o `dx` (ou a barreira) no CI e num ambiente de build
    controlado. Vê a
    [política de segurança](https://github.com/ngolacloud/dx/blob/main/docs/security.md)
    do projecto `dx`.

## Para onde ir a seguir

- [Gestão de pacotes](package-management.md) — os subcomandos `dx` reais para
  adicionar, remover, construir, correr, testar, bloquear e auditar.
- [dx vs Cargo](dx-vs-cargo.md) — uma comparação precisa comando a comando, o que
  o `dx` acrescenta, e quando recorrer a qual.
- [Usar o dx com o Seiva](with-seiva.md) — conduzir um projecto Seiva de ponta a
  ponta com o `dx`, e como ele assenta por baixo da CLI `seiva`.

---

*Parte da [documentação do Seiva](../index.md).*

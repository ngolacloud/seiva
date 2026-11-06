# Guia de instalação rápida

## Pré-requisitos

Uma toolchain estável e recente do Rust. Podes geri-la com `rustup` e o Cargo, ou
com [**`dx`**](https://ngolacloud.github.io/dx/) — o gestor de pacotes Rust da
Delonix ao estilo do UV, totalmente compatível com o Cargo (comanda o
`cargo`/`rustc` por baixo). Este guia mostra ambos; escolhe um. Uma base de dados é
opcional — um projecto gerado corre em memória até activares a camada de dados.

## Instalar a CLI

A CLI `seiva` é uma ferramenta global, por isso instala-a com o Cargo (até os
crates estarem publicados, a partir do workspace):

```console
$ cargo install --path crates/seiva-cli        # or, once published: cargo install seiva-cli
$ seiva --version
```

> **A usar o `dx`?** O `dx` é o gestor de pacotes do *projecto* (comanda o teu
> build e as dependências — ver abaixo). Para fixar o `seiva` como ferramenta
> local ao projecto e offline, em vez de uma instalação global, usa
> `dx tool install --path crates/seiva-cli` e corre-o com `dx tool run seiva …`.

## Criar e correr um projecto

```console
$ seiva project new demo
$ cd demo
$ seiva module new notes       # a module, auto-wired into the composition root
$ seiva start                  # run it (add --watch for hot-reload)
```

O serviço serve `/health`, `/openapi.json`, a documentação interactiva
(`/docs` = Swagger UI, `/redoc` = ReDoc, `/scalar` = Scalar) e o `POST /v1/notes`
do módulo. (`/admin` é uma extensão opcional — ver
[Documentos de API](../topics/api-documents.md); a base devolve `404` para ele.)
Verifica `seiva doctor` para a versão da framework e qual a camada de dados ligada.

## Verificar

```console
$ seiva openapi | grep '/v1/notes'      # the project's own contract
```

Depois abre <http://localhost:8080/docs> para a Swagger UI, onde podes experimentar
a chamada `POST /v1/notes` contra o serviço em execução.

## A seguir

Segue o [tutorial](tutorial/part-1-your-first-module.md), ou, se chegas de uma
framework dinâmica, começa pelo [teu primeiro dia](../first-day.md). O percurso
mais completo é o [quickstart](../quickstart.md).

---

*Parte da [documentação do Seiva](../index.md).*

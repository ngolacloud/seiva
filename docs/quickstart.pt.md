# Quickstart

Isto percorre o caminho de um checkout vazio até um serviço Seiva a correr. Tudo
aqui compila **sem** o repositório privado DORM — a camada de dados por omissão é
em memória até activares a feature `dorm`.

Pré-requisitos: uma toolchain Rust estável recente (`rustup`), e este workspace em
checkout.

## 1. Compilar e conhecer a CLI

Instala o binário `seiva` a partir do workspace:

```console
$ cargo install --path crates/seiva-cli
$ seiva doctor
Delonix Seiva
  framework version : 0.1.0
  data layer        : DORM (feature `dorm` on)
```

A CLI vem com a feature `dorm` activa por omissão; compila-a com
`--no-default-features` e o `doctor` reporta `external (feature `dorm` off)`.
(Ou corre sem instalar: `cargo run -q -p seiva-cli --bin seiva -- doctor`.)

`seiva --help` lista todos os comandos; os exercitados abaixo são `project`,
`module`, `openapi`, `proto` e `user`.

## 2. Ver os contratos gerados

A CLI imprime os documentos gRPC e OpenAPI gerados a partir da fachada de
demonstração embutida — os mesmos geradores que um módulo real usa:

```console
$ seiva proto
syntax = "proto3";
package seiva.notes.v1;
service Notes {
  rpc Create(CreateRequest) returns (CreateResponse);
}
...

$ seiva openapi | head
{
  "openapi": "3.1.0",
  ...
}
```

## 3. Criar um utilizador através do módulo accounts

`seiva user create` corre o caso de uso real do módulo `accounts`: lê a password
a partir do **stdin** (nunca da linha de comandos), faz o seu hash com Argon2id,
impõe as regras de domínio e emite o evento `accounts.user_created`. A saída
carrega o id e o username — nunca a password nem o seu hash.

```console
$ printf 'correct horse battery staple\n' | seiva user create --username alice
seiva: event emitted — accounts.user_created
{
  "id": "1793783057320429770",
  "username": "alice"
}
```

A persistência é em memória durante o tempo de vida do comando; torna-se durável
assim que um repositório DORM for ligado por trás da feature `dorm`.

## 4. Esqueletizar um projecto e um módulo

```console
$ seiva project new shop
$ cd shop
$ seiva module new catalog
```

Os nomes são em snake_case minúsculo (`shop`, `order_items`). `project new`
organiza `modules/`, `apps/shop-api`, `schema/`, `shared/kernel` e `seiva.toml`.
`seiva module new` emite depois as cinco camadas ligadas à framework:
`#[seiva::facade]` no contrato, `#[seiva::controller]` num adaptador de interface
concreto (o router `axum`), e helpers de interface que geram o OpenAPI e o
`.proto` do módulo a partir do seu descritor. O repositório por omissão é em
memória, por isso o projecto gerado compila sem base de dados.

## 5. Correr o servidor

O runtime `seiva-server` arranca a app HTTP (controllers + `/openapi.json` +
`/health`) e o host de workers, lendo a sua configuração 12-factor:

```console
$ SEIVA_HTTP_ADDR=127.0.0.1:8080 cargo run -q -p seiva-server --bin seiva-server
```

Precedência de configuração para o endereço de bind: `SEIVA_HTTP_ADDR` (completo
`host:port`) › `PORT` (porta nua, ligada em `0.0.0.0`) › `127.0.0.1:8080`.
`APP_ENV=production` activa a guarda de segurança de produção (atalhos de dev como
`SEIVA_EXPOSE_ERROR_DETAILS` são então rejeitados no arranque).

Noutra shell, os endpoints não guardados respondem de imediato:

```console
$ curl -s localhost:8080/health/ready        # 200 once Ready, 503 while Starting
$ curl -s localhost:8080/openapi.json | head  # the generated OpenAPI 3.1 document
```

Uma rota de fachada como `POST /v1/notes` é guardada pela sua permissão declarada
(`notes.create`): um chamador sem ela recebe um `403` RFC 9457. Envia `SIGTERM`
(ou Ctrl-C) e o servidor comuta a saúde para `Draining` e drena os pedidos em
curso antes de terminar.

## 6. Manter a arquitectura honesta

Corre as fitness functions localmente — as mesmas verificações que o CI impõe:

```console
$ seiva lint
```

Verificam a regra de dependência entre camadas e o contrato de zero-trust do
módulo, por isso uma violação falha cedo em vez de ser entregue.

## A seguir

- [Guia de arquitectura](architecture.md) — a estratificação, o contrato, e o
  papel de cada crate.
- Activar o DORM ponta-a-ponta exige o token de CI do repositório privado; vê a
  secção "DORM: por omissão, opcional e isolado" do guia de arquitectura.

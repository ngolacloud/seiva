# Parte 1: Um monólito modular

A loja começa a vida como **um binário** — um monólito modular. Obténs a
simplicidade de um único deploy e, como cada preocupação é um módulo separado com
o seu próprio contrato, mantens a opção de a separar mais tarde (Parte 7) sem uma
reescrita.

## Gerar o projecto

```console
$ seiva project new shop
$ cd shop
```

## Adicionar os cinco módulos

```console
$ seiva module new catalog
$ seiva module new inventory
$ seiva module new orders
$ seiva module new payments
$ seiva module new notifications
```

Cada `seiva module new` cria um módulo de cinco camadas sob `modules/<name>/` e
liga-o ao composition root. Depois dos cinco, o workspace fica assim:

```text
shop/
├── Cargo.toml
├── seiva.toml
├── apps/
│   └── shop-api/                # the composition root — the one binary
├── shared/
│   └── kernel/                  # ids, errors, time
├── modules/
│   ├── catalog/                 # products
│   ├── inventory/               # stock levels
│   ├── orders/                  # orders
│   ├── payments/                # payments
│   └── notifications/           # sent messages
├── schema/
│   └── shop.schema              # the DORM schema (Part 4)
├── Delonixfile                  # distroless image
└── compose.yaml                 # local run
```

Cada módulo são cinco crates (`facade`, `domain`, `application`, `infrastructure`,
`interface`) e o compilador impõe a regra de dependência entre eles — ver
[Arquitectura](../../architecture.md). Os módulos são **independentes**: nenhum
depende dos crates de outro módulo. Essa independência é o que torna a separação da
Parte 7 uma mudança de composição em vez de uma refactorização.

## Corre-o

```console
$ seiva start
```

O único binário serve `/health`, `/openapi.json`, a documentação interactiva e as
rotas de cada módulo sob `/v1/<module>`. A loja inteira é um documento OpenAPI, um
processo, um deploy — por agora.

```console
$ seiva openapi | grep -oE '"/v1/(catalog|inventory|orders|payments|notifications)[^"]*"' | sort -u
```

## A seguir

[Parte 2: Os módulos possuem os seus dados](part-2-modules-own-their-data.md) — os
cinco contratos e as permissões que declaram.

---

*Parte da [documentação do Seiva](../../index.md).*

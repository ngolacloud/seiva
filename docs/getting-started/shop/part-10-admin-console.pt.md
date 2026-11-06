# Parte 10: Uma consola admin, sem frontend escrito

Um back office precisa de uma UI de admin, e não deverias ter de construir uma à
mão. A **consola admin** do Seiva (`seiva-console-admin`) é uma **extensão** da base
(ADR 0008): monta-a e cada módulo que regista um recurso admin aparece, auto-CRUD,
guardado pelas mesmas permissões que a sua API. Não escreves frontend nenhum.

A consola é uma extensão, nunca uma dependência: um serviço construído sem ela não
tem rota `/admin` nem código de consola no binário. Aderes por opção.

## Registar um recurso num módulo

Um módulo expõe-se à consola registando um recurso admin — é isto que
`seiva module new --admin` monta, e o que cada módulo da loja já faz na sua camada
de interface:

```rust
// modules/catalog/interface/src/lib.rs
register_admin! {
    namespace: "catalog",
    resource:  "catalogs",
    label:     "Products",
    view:      "catalog.read",     // who may list/see it
    create:    "catalog.manage",   // who may add
}
```

A consola lê estes registos em tempo de link e mostra a um chamador **apenas** os
recursos para os quais possui a permissão `view` — a mesma regra de negação por
omissão que a API (Parte 5). As flags `can.*` apenas escondem affordances; o backend
continua a impor cada operação, por isso a consola nunca pode conceder um acesso que
o contrato negue.

## Montar a consola na raiz de composição

Em `apps/shop-api` a consola é fundida no router dos módulos *antes* das camadas da
framework, para que o autenticador preencha o `Principal` que a consola lê:

```rust
// apps/shop-api/src/main.rs
let modules = catalog_interface::catalog_router(/* … */)
    .merge(orders_interface::orders_router(/* … */))
    // … the other three modules …
    ;

let modules = seiva_console_admin::mount(modules);     // ← the extension

let app = router_with_modules(health, &HttpSecurity::default(), Some(auth), modules);
```

## Vê-la

```console
$ cargo run -p shop-api          # http://127.0.0.1:8090
# open http://127.0.0.1:8090/admin and enter a bearer token
```

Com um token que possua `catalog.read`/`catalog.manage` (e as permissões dos outros
módulos), a consola lista Products, Orders, Inventory, Payments e Notifications,
cada um com uma tabela de dados e formulários de criar/editar — gerados a partir
dos registos e do documento OpenAPI, servidos sob uma Content-Security-Policy
estrita (sem scripts inline, cada valor escapado).

## Onde vive o código

```text
modules/<module>/interface/src/lib.rs   # register_admin! { … }
apps/shop-api/src/main.rs               # seiva_console_admin::mount(modules)
```

## Construir e testar

```console
$ cargo run -p shop-api &
$ curl -s -o /dev/null -w '%{http_code}\n' -H 'authorization: Bearer <token>' localhost:8090/admin   # 200
```

A framework entrega um E2E de navegador (`e2e/admin`) que se autentica, lista o
recurso autorizado e conduz um formulário de criação de ponta a ponta em Chrome
headless — o padrão a copiar para os teus próprios testes de admin.

## Live-reload enquanto trabalhas

```console
$ seiva start --watch
```

## Seguinte

[Parte 11: Um frontend dxjs separado, sobre CORS](part-11-a-separate-dxjs-frontend.md)
— quando queres uma montra à medida em vez de (ou a par de) a consola admin.

---

*Parte da [documentação do Seiva](../../index.md).*

# A aplicação e a raiz de composição

A **raiz de composição** é `apps/<name>-api` — o `main` do serviço. É o único sítio
que escolhe adaptadores concretos e monta os módulos numa aplicação em execução.
Não contém lógica de negócio.

## O que o `main` faz

```rust
#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    observability::init();                       // structured JSON logs
    let config = ServerConfig::from_process_env()?;   // 12-factor config
    let health = Health::starting();
    let security = HttpSecurity::default();
    let auth = None;                             // or an Authenticator (OIDC/JWT)
    let modules = Router::new();                 // module routers merged here
    // seiva:module-merges (managed by the seiva CLI)
    let app = router_with_modules(health.clone(), &security, auth, modules);
    serve_router(config.http_addr, app, health, shutdown_signal()).await
}
```

As peças de runtime vêm de `seiva-server`: o serve loop, a
[configuração](../configuration.md) 12-factor, a [observabilidade](../observability.md)
e um esvaziamento gracioso em `SIGTERM` (o health passa de `Ready → Draining`, os
pedidos em curso terminam). A superfície do framework — `/health`,
`/openapi.json`, `/docs` — é montada por `router_with_modules`. A consola de
administração (`/admin`) **não** faz parte desta superfície: é uma extensão
opcional (`seiva-console-admin`, ADR 0008) que uma raiz de composição monta
explicitamente com `seiva_console_admin::mount(app)`.

## Fundir módulos (ligado automaticamente)

O router gerado de cada módulo é fundido em `modules` **antes** de as camadas do
framework o envolverem (a regra em [middleware e camadas](middleware-layers.md)).
Não fazes isto à mão: `seiva module new` insere a fusão (e as dependências do
módulo) nas âncoras geridas, escolhendo os adaptadores em memória:

```rust
let modules = modules.merge(notes_interface::notes_router(
    notes_interface::NotesController::new(
        notes_infrastructure::InMemoryRecordRepository::new(),
        notes_infrastructure::CounterIdGenerator::default(),
    ),
));
```

Para ires para a camada de dados DORM, troca os adaptadores em memória pelos
suportados por DORM por detrás da feature `dorm` (ver
[a camada de dados](../data-layer/index.md)); a raiz de composição é o único sítio
que nomeia um adaptador concreto.

## Emitir contratos

O mesmo binário pode imprimir o seu próprio contrato em vez de servir:
`SEIVA_EMIT=openapi` (ou `proto`) fá-lo emitir o documento a partir dos seus
módulos ligados e sair — que é como `seiva openapi`/`seiva proto` documentam o
*teu* projecto (ver [documentos de API](../api-documents.md)).

## Relacionado

- [Middleware e camadas](middleware-layers.md) · [Configuração](../configuration.md)
- [Controladores e encaminhamento](controllers-and-routing.md) · [Observabilidade](../observability.md)

---

*Parte da [documentação do Seiva](../../index.md).*

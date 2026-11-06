# Logging e observabilidade

Um serviço Seiva é observável por omissão: logs estruturados, um id de correlação em
cada pedido, e spans de tracing — configurados pelo runtime, não por cada equipa.

## Logs estruturados

A raiz de composição chama `observability::init()` **primeiro**, antes de tudo o
resto, instalando logging JSON estruturado para o stdout:

```rust
observability::init();
tracing::info!(env = %config.env, http = %config.http_addr, "notes-app starting");
```

Os logs são JSON no stdout — exactamente o que uma plataforma de containers recolhe.
Usa as macros do `tracing` (`info!`, `warn!`, `error!`) com campos estruturados
(`%value` para Display, `?value` para Debug); evita registar segredos ou corpos de
pedido.

> Nota: o caminho `SEIVA_EMIT` (`seiva openapi`/`proto`) corre *antes* de
> `observability::init()` para que o documento emitido seja a única coisa no stdout.

## Correlação de pedidos

A camada `request_id` mais externa atribui a cada pedido um id de correlação e
devolve-o num cabeçalho de resposta (`REQUEST_ID_HEADER`). Por ser a camada mais
externa, **todas** as respostas o carregam — incluindo as curto-circuitadas (um
`429`, um `401`, um preflight CORS). Leva esse id para os teus logs para rastrear um
pedido de ponta a ponta. Ver [middleware e a pilha de camadas](transport/middleware-layers.md).

## Spans de tracing

O framework e o runtime de workers emitem spans de `tracing` (custo quase nulo quando
não há subscritor instalado). O `observability::init()` instala um subscritor;
aponta-o ao teu colector em produção. Os spans dão-te latência e causalidade sem
cronometragem feita à mão.

## Saúde como sinal operacional

`/health/live` e `/health/ready` expõem o ciclo de vida
`Starting → Ready → Draining`. Ao `SIGTERM` o serviço muda para `Draining` e termina
os pedidos em curso (encerramento gracioso), pelo que um rollout drena limpamente.
Ver [a aplicação e a raiz de composição](transport/application-and-composition.md).

## Relacionado

- [Middleware e a pilha de camadas](transport/middleware-layers.md) · [Configuração](configuration.md)
- [Desempenho e cloud-native](performance.md) · [Segurança](security.md)

---

*Parte da [documentação do Seiva](../index.md).*

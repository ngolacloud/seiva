# A camada de transporte

> Esta é a **camada de transporte** do Seiva — a superfície HTTP/MCP, onde nunca
> escreves uma view ou uma rota de URL à mão. O transporte é *gerado* a partir do
> contrato da fachada.

## O fluxo

```
facade trait (#[seiva::facade])            ← the single source of truth
      │  generates FacadeDescriptor (name, version, methods, requires)
      ▼
#[seiva::controller] on the impl           → <trait>_router(service): axum::Router
      │
composition root (apps/<name>-api)         → router_with_modules(health, &security, auth, modules)
      ▼
seiva-server serve loop                     → the running service (+ /health, /openapi.json, /docs)
```

## Nesta secção

- [Fachadas: o contrato](facades.md) — declarar o trait versionado; o descriptor
- [Controladores e encaminhamento](controllers-and-routing.md) — a convenção
  `create`/`get`/`list`, o router gerado, permissões por rota
- [Pedidos e respostas](request-and-response.md) — o ciclo de vida, os `Problem`s RFC 9457, códigos de estado
- [Middleware e a pilha de camadas](middleware-layers.md) — request-id, cabeçalhos
  de segurança, authn, rate limit — e a ordem que os faz aplicar às rotas dos módulos
- [A aplicação e a raiz de composição](application-and-composition.md) —
  `router_with_modules`, health, o serve loop, encerramento gracioso

## A convenção de encaminhamento

O nome do método escolhe o verbo e o caminho (o recurso é o nome da fachada, em
minúsculas e no plural):

| método | HTTP | caminho | corpo |
| --- | --- | --- | --- |
| `create` | `POST` | `/{version}/{resource}` | o DTO (201 em sucesso) |
| `get` | `GET` | `/{version}/{resource}/{id}` | — (id no caminho) |
| `list` | `GET` | `/{version}/{resource}` | — (consulta opcional) |
| *outro* | `POST` | `/{version}/{resource}/{name}` | o DTO |

Um corpo de escrita é desserializado **directamente como o DTO do parâmetro**
(`Valid<T>`), e o documento OpenAPI reflecte exactamente isso — ver
[documentos de API](../api-documents.md).

## A única subtileza que vale a pena conhecer

O `.layer` do axum envolve apenas as rotas já presentes. Por isso, fundir um router
de módulo *depois* de aplicar as camadas do framework deixaria as rotas do módulo
**fora** de authn, dos cabeçalhos de segurança e do rate limit.
`router_with_modules` funde os módulos **antes** de aplicar as camadas — a única
ordem que aplica as garantias do framework às rotas dos módulos. Ver
[Middleware e camadas](middleware-layers.md).

---

*Parte da [documentação do Seiva](../../index.md).*

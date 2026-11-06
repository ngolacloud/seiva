# Guias temáticos ("Usar o Seiva")

Estes guias são **orientados à compreensão**: cada um discute uma camada ou
preocupação em profundidade, explicando não só o *como* mas o *porquê*. São os
guias temáticos "Usar o Seiva".

## As camadas

- **[A camada de dados (DORM)](data-layer/index.md)** — modelos, consultas,
  migrações, transacções, múltiplas bases de dados.
- **[A camada de transporte](transport/index.md)** — fachadas, controladores,
  encaminhamento, pedidos/respostas, a pilha de middleware, composição.
- **[Validação e DTOs](validation.md)** — entrada validada e erros RFC 9457.
- **[Documentos da API](api-documents.md)** — OpenAPI, a UI dos docs e proto. (O
  Seiva é API-first: gera os próprios documentos da API em vez de templates HTML.)

## Identidade, segurança, correcção

- [Autenticação](authentication.md) — um `Principal` verificado, OIDC/JWT, negação por omissão
- [Autorização](authorization.md) — ao nível do objecto, `Scoped<T>` fail-closed (BOLA)
- [Segurança](security.md) — a postura segura por omissão (cabeçalhos, limitação de taxa, OWASP API Top 10)
- [Testes](testing.md) — fakes, testes de controlador, funções de fitness arquitectural

## Operar um serviço

- [Configuração](configuration.md) — config 12-factor tipada e a guarda AppEnv
- [Observabilidade](observability.md) — logs estruturados, correlação, tracing
- [Desempenho e cloud-native](performance.md) — binário único, distroless, probes

## Ferramentas transversais

- [Mensageria e eventos](messaging-and-events.md) — CloudEvents, publicadores, a outbox
- [Despacho CQRS](cqrs.md) — mediação comando/consulta com middleware tipado
- [Workers em segundo plano](background-workers.md) — trabalho para além do pedido
- [Caching](caching.md) · [Paginação](pagination.md) · [Envio de email](sending-email.md)
  · [Sessões](sessions.md) · [Serialização](serialization.md)

---

*Parte da [documentação do Seiva](../index.md).*

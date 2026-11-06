# Referência de crates

O Seiva é um workspace de crates focados. A API completa ao nível dos tipos é
gerada por `cargo doc --workspace --no-deps`; esta página é o mapa do que cada
crate detém.

## O framework

| Crate | Detém |
| --- | --- |
| `delonix-seiva` | O ponto de entrada de que as apps a jusante dependem: o descritor `contract` + `schema` (`WireSchema`), as macros `facade`/`controller`/`WireSchema` (reexportadas), o runtime de workers, e uma reexportação das portas. |
| `seiva-ports` | A fronteira só-de-traits contra a qual a camada de aplicação codifica: `UnitOfWork`, `EventPublisher`, `Clock`, `IdGenerator`. Sem runtime, sem dependências. |
| `seiva-macros` | As macros procedurais: `#[facade]`, `#[controller]`, `#[derive(WireSchema)]`. |
| `seiva-http` | O transporte HTTP: `router`/`router_with_modules`, a pilha de camadas (request-id, cabeçalhos de segurança, autenticação, rate limit, CORS), o `Problem` da RFC 9457, `Valid<T>`, a guarda de permissões + `Scoped<T>`, saúde, `/docs`. (Sem `/admin`: isso é a extensão opcional `seiva-console-admin`.) |
| `seiva-console-admin` | **Extensão opcional (ADR 0008), não uma dependência da base.** A consola de administração em `/admin`, gerada a partir do contrato OpenAPI ao vivo, autocontida com uma CSP estrita baseada em hash. Uma raiz de composição monta-a com `seiva_console_admin::mount(router)`; um backend construído sem ela não tem `/admin`. |
| `seiva-openapi` | O gerador de OpenAPI 3.1 a partir dos descritores de fachada (+ WireSchema). |
| `seiva-grpc` | O gerador de serviço proto3 a partir dos descritores de fachada. |
| `seiva-identity` | O `Principal` verificado e o `Authenticator` (OIDC/JWT via JWKS). Sem transporte. |
| `seiva-auth` | Primitivas de password/credenciais (Argon2id PHC). |
| `seiva-config` | `ServerConfig` tipado 12-factor + a guarda de produção `AppEnv`. |
| `seiva-cqrs` | O mediador de comando/consulta da camada de aplicação + middleware tipado. |
| `seiva-messaging` | O envelope CloudEvents + `Publisher` + backends de broker. |
| `seiva-webhooks` | Assinatura/verificação Standard Webhooks. |
| `seiva-server` | O runtime: o ciclo de serviço, observabilidade, encerramento gracioso, host de workers. |
| `seiva-testing` | Test doubles em memória para as portas (uma dev-dependency). |
| `seiva-cli` | O binário `seiva` instalável (scaffold, ciclo de dev, lint, db, openapi/proto). |

## A camada de dados (acoplada ao DORM, controlada por feature)

Estes acoplam-se ao DORM e são construídos por trás da feature `dorm` / por si
próprios:

| Crate | Detém |
| --- | --- |
| `seiva-data-dorm` | O gateway DORM: `connect` + as quatro portas sobre o DORM (o padrão Gateway — todo o acoplamento ao DORM vive aqui). |
| `seiva-outbox-relay` | O worker de relay outbox-transaccional → broker. |

Os módulos de exemplo (`examples/notes`, `examples/accounts`) mostram a forma de
cinco camadas que o scaffolder emite.

## Relacionado

- [Arquitectura](../../architecture.md) · [O descritor de contrato](../contract-descriptor.md)

---

*Parte da [documentação do Seiva](../../index.md).*

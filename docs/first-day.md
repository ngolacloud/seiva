# O teu primeiro dia no Delonix Seiva (vindo do FastAPI)

Esta página é para quem já pensa em FastAPI e quer orientar-se no Seiva num dia
de trabalho. Mapeia cada hábito do FastAPI para o seu equivalente no Seiva, diz o
que já existe hoje, e é honesta sobre o que ainda falta.

> Estado: *founding stage*. O que está marcado **✅ hoje** funciona na árvore
> atual; **⏳ em curso** está desenhado mas ainda não ligado ponta-a-ponta.

## O modelo mental

| No FastAPI | No Seiva | Porquê a diferença |
| --- | --- | --- |
| `@app.post("/notes")` numa função | Um método numa *facade* (`#[seiva::facade]`); a rota é gerada do descritor | O contrato é a fonte da verdade; o transporte é derivado, não escrito à mão |
| `pydantic.BaseModel` para o corpo | `Valid<T>` + `impl Validate` | Validação na entrada, erro `422` RFC 9457 (mais rigoroso que o default do FastAPI) |
| `Depends(get_db)` por rota | Portas (`UnitOfWork`, `IdGenerator`, `EventPublisher`, `Clock`) injetadas no construtor do serviço da camada `application` | Injeção estática, verificada em compilação, trocável por *test doubles* (`seiva-testing`) |
| `/docs` (Swagger UI) e `/redoc` automáticos | `GET /docs` (Swagger UI), `/redoc` (ReDoc) e `/scalar` (Scalar) sobre o `/openapi.json` do serviço | ✅ hoje |
| `/openapi.json` automático | `GET /openapi.json`, gerado de **todas** as fachadas montadas (descoberta em tempo de link) | ✅ hoje |
| `HTTPException` | `Problem` (RFC 9457 `application/problem+json`) | Formato padronizado, com `request_id` de correlação |

## Os cinco minutos iniciais

```bash
seiva project new demo        # ✅ gera workspace: seiva.toml, apps/demo-api, shared/kernel, modules/
seiva module new notes        # ✅ gera as 5 camadas (facade/domain/application/infrastructure/interface)
seiva lint                    # ✅ fitness function: recusa dependências proibidas entre camadas
seiva openapi                 # ✅ imprime o OpenAPI 3.1 das fachadas ligadas
seiva start                   # ⏳ arranca o servidor em modo dev com hot-reload (ligado em S3)
```

Depois de `seiva start` (S3), abre `http://localhost:8080/docs` para a referência
interativa e `http://localhost:8080/health/ready` para a prova de vida.

## O que já tens de produção que o FastAPI não dá por omissão

- **Erros RFC 9457** (`problem+json`) com `X-Request-Id` ponta-a-ponta.
- **Guarda de permissões** que **nega por omissão** (`permission_guard`): uma rota
  guardada sem principal responde `403`, nunca admite.
- **Posse por tenant** (`Principal::require_owns`) para autorização object-level (BOLA).
- **Headers de segurança** por omissão (CSP, HSTS, `nosniff`, `X-Frame-Options`) e
  **CORS** por allowlist explícita.
- **Rate limiting** por cliente com `Retry-After`.
- **Lockout de autenticação** (`seiva_auth::LoginThrottle`) e hashing **Argon2id**.
- **Health** `Starting/Ready/Draining` mapeável a `liveness`/`readiness` de Kubernetes,
  com **graceful shutdown** (SIGTERM → drena em curso).
- **Config 12-factor** validada no arranque, com guarda de atalhos de dev em produção.

## O que falta para o "primeiro dia" ficar completo

| Lacuna | Estado | Fase |
| --- | --- | --- |
| `seiva start` com hot-reload a arrancar o projeto | ⏳ desenhado | S3 (acoplado ao scaffold emitir um servidor real) |
| App gerado (`apps/<name>-api`) ser um servidor real (não um stub) | ⏳ | S3 |
| Middleware de autenticação de referência que **preenche** o `Principal` (JWT/sessão) | ⏳ | pós-S1 |
| `Valid<T>` com mapa de erros por-campo (hoje é mensagem única) | ⏳ | posterior |
| `seiva openapi` introspecionar as fachadas do **projeto do utilizador** (o runtime já as descobre; a CLI isolada só vê o que linka) | limitação inerente | documentada |

## Leitura a seguir

- `docs/quickstart.md` — o arranque passo a passo.
- `docs/architecture.md` — as 5 camadas, as portas e as fitness functions.
- `docs/progress.md` — o estado por fase, incluindo a auditoria de segurança.

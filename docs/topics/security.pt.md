# Segurança

A postura do Seiva é **segura por omissão** — mapeada sobre o OWASP **API Security
Top 10 (2023)**, a checklist certa para um framework API-first. A segurança é imposta
por construção (o sistema de tipos e a pilha de camadas), não deixada a cada equipa.

## Segura por omissão

Todo o serviço construído sobre `router_with_modules` ganha, sem configuração:

- **Autenticação por negação por omissão** à frente de todas as rotas (ver
  [Autenticação](authentication.md)) — o controlo mais importante, não feito à mão.
- **Autorização ao nível da função** — o `#[requires("…")]` de cada método de fachada
  é imposto pelo `permission_guard` a partir do descritor (OWASP API5).
- **Autorização ao nível do objecto** — o `Scoped<T>` torna o acesso entre
  tenants/entre donos uma preocupação de **tempo de compilação**, fail-closed por
  tipo (OWASP API1 / BOLA; ver [Autorização](authorization.md) e
  [ADR 0005](../adr/0005-object-level-authorization-bola.md)).
- **Cabeçalhos de resposta seguros** em *todas* as respostas — incluindo as
  curto-circuitadas (um `429`, um `401`, um preflight CORS), porque a camada de
  cabeçalhos é a mais externa.
- **Um limitador de taxa** e CORS opcional (mesma origem a menos que adiciones
  origens).
- **Um `request-id` de correlação** em cada pedido/resposta.
- **Entrada validada** → RFC 9457 `422` (ver [Validação](validation.md)); o ORM
  parametriza as consultas (sem injecção de SQL).
- **Protecção contra inventário impróprio (API9)** — o `/openapi.json` lista *todas*
  as fachadas montadas via descoberta em tempo de ligação, pelo que a superfície
  documentada corresponde à real.

## A ordem das camadas importa

Mais externa primeiro: `request_id` → `security_headers` → `authn` → limite de taxa →
CORS → limite de tamanho de corpo → rota. Os cabeçalhos e o request-id ficam **fora**
do authn para que um `401` ainda os carregue; o authn fica **fora** das rotas para
que corra antes da guarda de cada rota. E os módulos são fundidos **antes** da
camada (`router_with_modules`) — caso contrário as rotas de módulo escapariam a tudo
o acima. Ver [Middleware e camadas](transport/middleware-layers.md).

## OWASP API Security Top 10 (2023) — postura

| | Controlo no Seiva |
| --- | --- |
| API1 Broken Object Level Auth | `Scoped<T>`, fail-closed por tipo |
| API2 Broken Authentication | `seiva-identity`, JWT/OIDC verificado, negação por omissão |
| API3 Broken Object Property Level Auth | DTOs são tipos de fio explícitos; sem fuga de domínio |
| API5 Broken Function Level Auth | `#[requires]` + `permission_guard` |
| API7 SSRF | JWKS injectado, não obtido a partir da camada de verificação |
| API8 Security Misconfiguration | cabeçalhos seguros sempre ligados; guarda de produção AppEnv |
| API9 Improper Inventory | `/openapi.json` a partir da descoberta de fachadas em tempo de ligação |

Corre a checklist antes de qualquer PR que toque em `seiva-http`, `seiva-auth`,
`seiva-webhooks` ou o portão de segurança da CI — ver a skill `seiva-security-review`.

## Relacionado

- [Autenticação](authentication.md) · [Autorização](authorization.md)
- [ADR 0005 — BOLA](../adr/0005-object-level-authorization-bola.md) ·
  [ADR 0006 — Identidade](../adr/0006-identity-and-authentication.md)

---

*Parte da [documentação do Seiva](../index.md).*

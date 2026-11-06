# Autenticação

> O **sistema de autenticação** integrado do Seiva — verificação primeiro e
> agnóstico ao transporte.

Em vez de te entregar *primitivas* OAuth2/JWT e deixar o resto contigo, o Seiva traz
uma camada de identidade de **negação por omissão** que transforma um token bearer
num `Principal` criptograficamente verificado antes de correr a verificação de
permissão de qualquer rota. O desenho está registado na
[ADR 0006](../adr/0006-identity-and-authentication.md).

## Um `Principal`, um `Authenticator`

Uma única crate sem transporte, `seiva-identity`, detém o modelo para que HTTP e MCP
não possam derivar para auth incompatível:

```rust
pub struct Principal {
    pub subject: String,
    pub tenant: Option<String>,
    pub permissions: BTreeSet<String>,
}

pub trait Authenticator {
    fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError>; // deny by default
}
```

- **`TokenAuthenticator`** — um mapa estático, para dev e testes.
- **`JwtAuthenticator`** — verifica um OIDC/JWT com a crate auditada `jsonwebtoken`
  (sobre `ring`). Os algoritmos são uma **allowlist apenas de RS256 e ES256** — sem
  segredo partilhado HS\*, nunca `alg: none`. Valida `iss`, `aud`, `exp`, `nbf`
  contra um `JwkSet` injectado, e mapeia claims → `Principal` (`sub` → subject; uma
  claim de tenant configurável; permissões a partir de uma claim configurável, com
  suporte a `scope` delimitado por espaços).

O material de chave é **injectado, não obtido** aqui — a crate mantém-se sem rede e
deterministicamente testável; a obtenção/rotação de JWKS e a allowlist de issuers
(SSRF, OWASP API7) pertencem à camada de ligação.

## Como se comporta no fio

Montado à frente da guarda de permissão por `router_with_auth` /
`router_with_modules`:

| Pedido | Resultado |
| --- | --- |
| **Sem** bearer | tratado como `anonymous()` — rotas públicas servem, rotas guardadas `403` |
| Bearer **inválido** | `401` com `WWW-Authenticate` |
| Bearer **válido** | `Principal` a partir das claims verificadas; o `#[requires(..)]` da rota é verificado |

## Ligá-lo num serviço

O `seiva-server` lê a config OIDC do ambiente e constrói um `JwtAuthenticator` a
partir do ficheiro JWKS, montando `authn` por ti:

```
SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
SEIVA_OIDC_ISSUER=https://id.example.com/
SEIVA_OIDC_AUDIENCE=my-api
# optional: SEIVA_OIDC_TENANT_CLAIM, SEIVA_OIDC_PERMISSIONS_CLAIM
```

Um trio parcial falha o arranque (fail-closed). Provado ao vivo: sem token → `200`
(anónimo), bearer errado → `401`, um JWT RS256 válido → `200`.

## Relacionado

- [Autorização](authorization.md) — ao nível do objecto, `Scoped<T>` fail-closed (BOLA)
- [Segurança](security.md) — a postura mais ampla de segurança por omissão
- [Referência de configuração](../reference/settings.md) — as variáveis OIDC
- [ADR 0006 — Identidade e autenticação](../adr/0006-identity-and-authentication.md)

---

*Parte da [documentação do Seiva](../index.md).*

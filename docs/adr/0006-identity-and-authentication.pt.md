# ADR 0006 — Identidade e autenticação: um `Principal` verificado para cada transporte

- Estado: Aceite
- Data: 2026-11-04
- Decisores: NgolaCloud
- Segue: ADR 0003, ADR 0005

## Contexto

O Seiva impõe autorização ao nível da função (ADR 0003, `permission_guard`) e ao
nível do objecto (ADR 0005, `Scoped<T>`) — mas ambas lêem um `Principal` que
*algo a montante* deve ter produzido, e sobre HTTP **nada o faz**. O
`permission_guard` recorre a `Principal::anonymous()` quando não há `Principal`
nas extensões do pedido, por isso cada rota protegida só é alcançável por um
chamador autenticado que o framework nunca autentica. A auditoria de prontidão
(2026-11-04) fez disto o bloqueador de topo: o único controlo mais importante de
uma plataforma crítica fica para cada equipa artesanar à mão.

Mais duas forças moldam a resposta:

- Já existem **dois** tipos `Principal` — `seiva-http::guard::Principal`
  (subject, tenant, permissions) e `seiva-mcp::auth::Principal` (subject, tenant,
  can_write) com um trait `Authenticator` que já nega por omissão. Vão derivar
  para modelos de autenticação incompatíveis a menos que sejam unificados.
- Verificar um JWT é um campo minado de cripto (`alg: none`, confusão de
  algoritmo, `exp`/`aud` não validados). Um framework de referência não o pode
  artesanar à mão, e não pode alcançar a rede a partir de uma camada que tem de
  se manter testável.

## Decisão

**Uma crate `seiva-identity` sem transporte é dona do `Principal` e do trait
`Authenticator`; um `JwtAuthenticator` transforma um bearer num `Principal`
apenas a partir de um OIDC/JWT criptograficamente verificado; o middleware HTTP
monta-o à frente do `permission_guard`, negando por omissão.**

1. **`seiva-identity` (nova, sem transporte, como `seiva-ports`/`seiva-auth`).**
   Define o único `Principal { subject, tenant: Option<String>, permissions:
   BTreeSet<String> }` e o trait `Authenticator` (`fn authenticate(&self, bearer:
   &str) -> Result<Principal, AuthError>`, **negação por omissão**). `seiva-http`
   e `seiva-mcp` dependem dela e largam as suas cópias locais; `seiva-http`
   re-exporta `Principal`/`Scoped` para que a sua superfície fique inalterada. O
   `TokenAuthenticator` estático move-se para cá como a implementação de
   dev/teste.

2. **`JwtAuthenticator` verifica com a crate auditada `jsonwebtoken` (sobre
   `ring`).** Os algoritmos são uma allowlist de **apenas RS256 e ES256** — sem
   segredo partilhado HS*, nunca `alg: none`. Valida `iss`, `aud`, `exp` e `nbf`
   (pequena folga) contra um `jsonwebtoken::jwk::JwkSet` injectado. O mapeamento
   claim → Principal é configurado: `sub` → subject; um claim de inquilino
   configurável (por omissão `tenant`); permissões a partir de um claim
   configurável (por omissão `permissions` como array de strings, com `scope`
   delimitado por espaços suportado).

3. **O material de chave é injectado, não obtido aqui.** O `JwtAuthenticator`
   guarda o `JwkSet` por valor. Obtê-lo do `jwks_uri` do emissor e rotá-lo é uma
   preocupação separada (uma futura porta de refresh), mantida de fora para que a
   crate fique sem rede e deterministicamente testável (injectar um conjunto de
   chaves gerado localmente). Isto também adia a preocupação de
   SSRF/allowlist-de-emissor (auditoria API7) para a camada de ligação, em vez de
   embeber um cliente.

4. **Negação por omissão, mas as rotas públicas continuam a servir.** O
   middleware HTTP, montado à frente do `permission_guard`: um bearer **em
   falta** → `anonymous()` (para que `/health`, `/docs`, `/openapi.json` fiquem
   públicos e as rotas protegidas 403 via o guard existente); um bearer
   **presente-mas-inválido** → `401` RFC 9457 `Problem` com `WWW-Authenticate:
   Bearer`. O guard mantém-se o portão de negação por omissão por rota.

## Alternativas consideradas

- **Manter `Principal`/autenticação por transporte (status quo).** Rejeitada: os
  dois tipos `Principal` derivam, e o HTTP não tem autenticador nenhum — o
  bloqueador.

- **Segredo partilhado simétrico HS256.** Rejeitada: um segredo partilhado por
  cada serviço, sem rotação de chave, sem confiança assimétrica. Uma plataforma
  crítica multi-serviço precisa de verificação JWKS/assimétrica onde só o IdP
  pode cunhar.

- **Artesanar a verificação de JWT à mão.** Rejeitada: `alg: none` e a confusão
  de algoritmo são exactamente os tiros no pé que um framework de referência não
  pode entregar; usar o auditado `jsonwebtoken` + `ring`.

- **Tokens opacos + introspecção, ou cookies de sessão.** Rejeitada *para o
  primeiro corte*: com estado, precisa de store/ida-e-volta. JWT/JWKS é sem
  estado e encaixa no runtime 12-factor. Ambos continuam possíveis mais tarde
  como outra implementação de `Authenticator` por trás do mesmo trait — que é o
  propósito do trait.

- **Rejeitar tokens em falta com 401 globalmente no middleware.** Rejeitada: mata
  as rotas públicas. A negação por omissão já é imposta por rota pelo
  `permission_guard`; em falta → anónimo é a divisão correcta.

## Consequências

- **Mais fácil:** um só `Principal` através de HTTP, MCP e qualquer transporte
  futuro (gRPC); trocar o IdP é trocar o `Authenticator`; as camadas de
  autorização (ADR 0003, 0005) têm finalmente uma fonte de confiança para
  subject/tenant/permissions.

- **Dívida aceite (2026-11-04):** a obtenção + rotação do JWKS e a allowlist de
  emissor (SSRF, auditoria API7) ficam adiadas — a crate consome um `JwkSet`
  estático até uma porta de refresh aterrar. A *emissão* de tokens fica de fora do
  Seiva por completo (trabalho de um IdP). Registado em `docs/progress.md`.

- **Camadas:** `seiva-identity` é sem transporte; `seiva-http` e `seiva-mcp`
  dependem dela. A sua relação com `seiva-auth` (hashing de passwords,
  `Permission`) é de referência cruzada, não de fusão — `seiva-auth`
  emite/verifica *credenciais*, `seiva-identity` verifica *tokens* e modela o
  chamador.

- **Imposição:** a allowlist de algoritmos (RS256/ES256) e a validação
  obrigatória de `exp`/`aud`/`iss`; uma matriz de testes de assinatura-má /
  `aud`-errado / expirado / `alg:none` todos a não produzir `Principal`; o guard
  mantém-se de negação por omissão; e uma fitness check de que nenhum exemplo
  injecta `Principal::new` fora de `#[cfg(test)]`.

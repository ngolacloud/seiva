# ADR 0040 — O fluxo de login OIDC completo com rotação automática de JWKS (estende o `seiva-identity`)

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade e autenticação), ADR 0019 (autorização fail-closed), ADR 0038 (authorization server OAuth 2.1)

## Contexto

O `seiva-identity` verifica um bearer, mas não **autentica um utilizador**. O ADR
0006 traçou a linha de propósito: o `JwtAuthenticator` guarda um `JwkSet` **por
valor**, e «buscá-lo do `jwks_uri` do issuer e rotacioná-lo é uma preocupação
separada (um futuro refresh port), deixada de fora para o crate ficar network-free
e deterministicamente testável». Esse adiamento é agora a lacuna aberta. Um
`JwkSet` estático significa que, quando um IdP externo rotaciona as suas chaves de
assinatura — o que todo o IdP faz, por calendário — cada token assinado pela nova
chave é rejeitado até um humano reinjectar o conjunto de chaves. Em produção isso é
uma indisponibilidade. O composition root ficou encarregue de resolver, e entre
produtos resolve-se de forma diferente ou nem se resolve.

O *driver* precisa de mais do que verificação: o delonix-meet autentica utilizadores
**através de um IdP externo** (o seu SSO Odoo é a referência — a org e o directório
de um utilizador nascem do 1º login). Esse é o fluxo completo de **OpenID Connect**
como relying party: descobrir o IdP, redireccionar para ele, receber e validar um
`id_token`, mapear os seus claims para um `Principal`, e fazer **just-in-time
provisioning** da org/utilizador à primeira vista. A *metade* authorization-server é
o ADR 0038; este ADR é a metade *relying party* e, criticamente, **fecha a lacuna de
rotação de JWKS que o ADR 0006 deixou aberta**.

As forças:

- **A rotação de JWKS tem de ser automática e resiliente.** A peça em falta é uma
  cache indexada por `kid` que se refresca por calendário e num miss de `kid`
  desconhecido — nunca um fetch por pedido (uma armadilha de latência e de
  amplificação de DoS), nunca um conjunto estático que fica obsoleto. Quando uma
  chave roda, a verificação tem de continuar a funcionar sem um humano.
- **O login é uma cerimónia, a verificação é uma primitiva.** O fluxo de relying
  party (discovery → authorize+PKCE → callback → validação do `id_token` → mapeamento
  de claims) é mecanismo horizontal; *que* IdPs são confiáveis, *como* os claims
  mapeiam para papéis, e *que* org um novo login integra são a política do produto.
- **JIT provisioning, fail-closed.** Num primeiro login válido, a org e o utilizador
  são criados a partir dos claims verificados (o padrão SSO-Odoo) — mas só através de
  uma regra do produto, nunca confiando num claim não validado para conceder tenancy.
- **Só algoritmos assimétricos, como no ADR 0006.** Só RS256/ES256, nunca `HS*` ou
  `none`; `iss`/`aud`/`exp`/`nbf` validados; `nonce` e `state` vinculam o round-trip;
  o JWKS é buscado por TLS de um issuer em **allowlist** (a preocupação de SSRF que o
  ADR 0006 marcou para a camada de wiring, agora assumida aqui).
- **Fail-closed quando o JWKS está indisponível.** Se um refresh falha *e* o `kid` do
  token é desconhecido, a verificação é **recusada** — uma chave em cache obsoleta mas
  válida ainda pode servir, mas uma chave desconhecida com um JWKS morto nunca é
  deixada passar.

## Decisão

Estender o **`seiva-identity`** com um fluxo de relying party OIDC e uma cache de
JWKS automática e resiliente — o `JwtAuthenticator` continua a funcionar a partir de
um conjunto estático, e uma nova camada acrescenta o fetch+rotação e a cerimónia de
login por cima.

- **Discovery** — um `DiscoveryClient` lê `/.well-known/openid-configuration` de um
  issuer em **allowlist** (só TLS), cacheando os endpoints `jwks_uri`, `authorization`
  e `token`; o documento é cacheado, não re-buscado por login.
- **Rotação automática de JWKS (a lacuna fechada)** — um `JwksCache` indexado por
  `kid` que: (a) refresca por TTL; (b) num miss de verificação para um **`kid`
  desconhecido**, dispara um único refresh limitado (coalescido, para que uma rajada
  seja um fetch); (c) serve **stale-while-revalidate** quando o issuer está
  brevemente inalcançável; (d) **falha fechado** quando o `kid` é desconhecido *e* o
  refresh falha. Isto substitui o `JwkSet` por-valor do ADR 0006 por uma fonte de
  chaves viva e auto-reparadora — com um port `JwksSource` para os testes injectarem
  um conjunto determinista e nenhuma rede.
- **Login authorization-code + PKCE** — um `begin_login` que constrói o URL de
  authorize (com `state`, `nonce`, `code_challenge` PKCE) e um `complete_login` que
  trata o callback: verifica o `state`, troca o code no token endpoint, e valida o
  **`id_token`** devolvido (assinatura via o `JwksCache`, `iss`/`aud`/`exp`/`nbf`, e o
  `nonce` contra o emitido no `begin_login`).
- **Mapeamento claim → `Principal`** — os claims validados do `id_token`/`userinfo`
  mapeiam para o único `Principal { subject, tenant, permissions }` (ADR 0006) através
  de um mapeamento configurado pelo produto (que claim é o subject, o tenant, os
  papéis).
- **JIT provisioning** — um port `ProvisioningHook` que o produto implementa: num
  subject visto pela primeira vez, cria a entrada de org/utilizador/directório (o
  padrão SSO-Odoo «org nascida do 1º login») sob as **regras de entrada de org** do
  produto — o framework nunca concede tenancy a partir de um claim cru.

O cliente de rede entra atrás de uma feature; o `JwksSource`/`DiscoveryClient`/
`ProvisioningHook` são ports com duplos em memória, para que o fluxo de login fique
testável em unidade sem um IdP vivo. O `seiva-oauth` (ADR 0038) pode ele próprio ser
o IdP, fechando o ciclo para o SSO de 1ª parte.

## Alternativas consideradas

- **Manter o `JwkSet` estático e reinjectar na rotação (status quo, ADR 0006).**
  Rejeitado: isso é uma indisponibilidade agendada à espera de acontecer — o IdP
  rotaciona, os tokens falham, um humano corre a apagar o fogo. A rotação automática é
  o mínimo para qualquer SSO real.
- **Buscar o JWKS por pedido.** Rejeitado: acrescenta a latência do IdP a cada
  chamada e torna o IdP num ponto único de falha e num amplificador de DoS. Uma cache
  indexada por `kid` com stale-while-revalidate é ao mesmo tempo mais rápida e mais
  resiliente.
- **Refrescar a cache mas falhar *aberto* quando o JWKS está em baixo.** Rejeitado em
  absoluto: aceitar um token cuja chave não se consegue verificar é o pecado cardeal
  da autenticação. As chaves em cache obsoletas podem servir; um `kid` desconhecido
  com um JWKS morto falha fechado (ADR 0019).
- **Construir o fluxo de login em cada produto por cima do verificador do
  `seiva-identity`.** Rejeitado: isso volta a derivar discovery, PKCE, vínculo de
  `nonce`/`state` e — o mais perigoso — a rotação de JWKS por produto, cada um
  subtilmente diferente. O propósito todo de o ADR 0006 ter deixado um «futuro refresh
  port» era aterrá-lo aqui, uma vez.

## Consequências

- **Performance.** O JWKS e os documentos de discovery são **cacheados**, não buscados
  por pedido (fechando o custo implícito do ADR 0006); os refreshes são coalescidos
  para que uma rajada de troca de chave seja um único fetch; o caminho quente de
  verificação continua a ser o `JwtAuthenticator` em memória. Zero-cost para um
  serviço que não faz SSO.
- **Segurança.** Só RS256/ES256 (nunca `HS*`/`none`); `iss`/`aud`/`exp`/`nbf`, `nonce`
  e `state` todos validados; o JWKS e o discovery são buscados por **TLS** de um
  issuer em **allowlist** (a preocupação de SSRF que o ADR 0006 adiou, agora assumida);
  o JIT provisioning concede tenancy só através da regra de entrada de org do produto,
  nunca de um claim cru. Fail-closed de ponta a ponta (ADR 0019).
- **Resiliência.** A rotação de JWKS é **resiliente**: refresh por TTL + refresh no
  miss de `kid` desconhecido + **stale-while-revalidate** + retry limitado mantêm a
  verificação a funcionar através de uma troca de chave do IdP sem humano; e quando o
  `kid` é desconhecido *e* o JWKS está inalcançável, **falha fechado** em vez de
  confiar num token não verificável — transformando a «dívida aceite» do ADR 0006 numa
  fonte de chaves auto-reparadora.
- O delonix-meet reusa o fluxo de login e monta as suas regras de entrada de org do
  SSO-Odoo como a política do `ProvisioningHook` (org e directório nascidos do 1º
  login), apagando a sua canalização de discovery/rotação feita à mão; toda a futura
  plataforma Seiva ganha SSO externo resiliente como uma configuração, não um build.
- **Não** torna o Seiva num IdP — isso é o authorization server do ADR 0038; este ADR
  é o **relying party** que consome qualquer IdP OIDC (incluindo o do próprio Seiva) e
  finalmente fecha a lacuna de rotação de JWKS que o ADR 0006 abriu.

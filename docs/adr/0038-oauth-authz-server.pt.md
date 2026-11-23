# ADR 0038 — `seiva-oauth`: um authorization server OAuth 2.1 (o Seiva hoje só verifica JWTs)

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0005 (autorização ao nível do objecto / BOLA), ADR 0006 (identidade e autenticação), ADR 0019 (autorização fail-closed), ADR 0035 (refresh tokens rotativos)

## Contexto

O Seiva verifica tokens; não os **emite**. O `seiva-identity` transforma um bearer
num `Principal` apenas a partir de um OIDC/JWT criptograficamente verificado
(RS256/ES256, com `iss`/`aud`/`exp`/`nbf` validados), e o ADR 0006 afirma que a
*emissão* de tokens «fica inteiramente fora do Seiva (é tarefa de um IdP)». Foi o
primeiro corte certo: um framework não deve improvisar um authorization server à
toa. Mas o *driver* força a questão. O delonix-meet expõe uma superfície **MCP** e
um cliente web de 1ª parte, e ambos precisam de tokens *de 1ª parte* emitidos
contra os utilizadores, clientes e scopes da própria plataforma — não há IdP
externo que conheça «os lugares e papéis deste tenant da plataforma de reuniões».
Hoje essa lacuna é tapada à mão, por produto, que é exactamente a forma que o
princípio do upstreaming manda levantar uma vez, correctamente.

Um authorization server é horizontal: *qualquer* plataforma que tenha os seus
próprios clientes (um servidor MCP, uma SPA, uma app móvel, uma integração
serviço-a-serviço) precisa de emitir, dar scope, introspeccionar e revogar tokens
para eles. O domínio fica no produto: *que* scopes existem e *o que* cada um
protege é política; a **cerimónia** OAuth é mecanismo.

As forças:

- **OAuth 2.1, não 2.0.** Os footguns obsoletos são removidos por construção: sem
  implicit flow, sem password grant, **o PKCE é obrigatório** em cada troca de
  authorization-code (clientes públicos *e* confidenciais), correspondência
  exact-string do redirect-URI. Entregar a forma de PKCE-opcional do 2.0 seria
  reimportar as vulnerabilidades que um framework de referência não pode trazer.
- **A emissão passa a ser uma preocupação do Seiva, a verificação já é.** O server
  emite os seus próprios JWTs (ES256/RS256) e publica o seu próprio JWKS; o
  `seiva-identity` verifica-os com o código que já tem (ADR 0006). Uma raiz de
  confiança, duas metades.
- **Deny-by-default, em todo o lado.** Os scopes são uma allowlist por cliente; um
  scope não pedido ou não concedido fica ausente, nunca implícito (ADR 0005/0019).
  Os authorization codes são de curta duração, de uso único, vinculados ao cliente
  e ao challenge PKCE. Um code reutilizado ou expirado é recusado.
- **O refresh já está resolvido.** O ADR 0035 é dono dos refresh tokens rotativos,
  revogáveis e com detecção de reutilização; o token endpoint emite-os e rotaciona-os
  através desse crate em vez de voltar a derivar o ciclo de vida.
- **Zero-cost quando não usado e DORM-free.** Um serviço que não emite tokens não
  liga nenhum authorization server. Os stores de cliente/consentimento/grant ficam
  atrás de ports, como qualquer outra costura de persistência do Seiva.

## Decisão

Adicionar um novo membro do workspace **`seiva-oauth`** — um authorization server
OAuth **2.1**, sem camada de dados, atrás de feature, composto na raiz.

- **Endpoint de autorização** — `GET /oauth/authorize`: valida o `client_id`, o
  `redirect_uri` por **correspondência exacta**, o subconjunto de `scope` pedido, o
  `state` e (para OIDC, ADR 0040) o `nonce`, exige um `code_challenge` **PKCE** +
  `code_challenge_method=S256`, renderiza um **ecrã de consentimento** e, na
  aprovação, emite um **authorization code** de curta duração e uso único, vinculado
  ao cliente, ao redirect-URI e ao challenge PKCE.
- **Token endpoint** — `POST /oauth/token`: o grant `authorization_code`
  (verificando o `code_verifier` do PKCE, o uso único e o vínculo do code) e o grant
  `refresh_token` (delegado ao ADR 0035). Suporta também o grant
  `client_credentials` para chamadores serviço-a-serviço. **Emite um JWT** de acesso
  (ES256/RS256) com os scopes concedidos e os claims de subject/tenant que o
  `seiva-identity` já lê.
- **Registo de clientes** — um port `ClientStore` sobre clientes **confidenciais**
  (com um client secret em hash) e **públicos** (só PKCE), cada um carregando a sua
  allowlist exacta de redirect-URI e os scopes que pode pedir. O registo dinâmico é
  um incremento posterior, não este corte.
- **Scopes como política de domínio** — o produto declara o seu catálogo de scopes;
  o server trata os scopes como strings opacas em allowlist, deny-by-default, e
  carimba só o subconjunto concedido no token.
- **Introspection + revocation** — `POST /oauth/introspect` (RFC 7662) responde
  active/scope/subject para tokens opacos ou de referência; `POST /oauth/revoke`
  (RFC 7009) revoga um refresh ou access token, passando pelo store do ADR 0035.
- **Chaves de assinatura próprias + JWKS** — um `SigningKeyStore` guarda a chave de
  assinatura actual e a anterior; o server publica `GET /oauth/jwks.json` com um
  `kid` estável por chave, para que os verificadores (incluindo o `seiva-identity`,
  ADR 0040) rotacionem sem downtime.

O `tokio` entra com `default-features = false`; nenhum runtime é ligado à
biblioteca. Os stores são ports com duplos em memória para testes e adapters
suportados por DORM atrás da feature `dorm`.

## Alternativas consideradas

- **Manter a emissão fora do Seiva (status quo, ADR 0006).** Rejeitado agora, não
  então: o *driver* precisa de tokens de 1ª parte para os seus clientes MCP e web, e
  «cada produto improvisa um authorization server» volta a derivar o código mais
  perigoso da pilha por equipa. A costura de trait que o ADR 0006 deixou aberta é
  exactamente onde isto aterra.
- **Implementar OAuth 2.0 com PKCE opcional.** Rejeitado: os grants implicit e
  password do 2.0 e o PKCE opcional são as funcionalidades removidas por causa
  justa. Um framework de referência entrega o perfil seguro do 2.1 ou nada.
- **Só tokens opacos + introspection (sem emissão de JWT).** Rejeitado como
  default: força um round-trip de rede por pedido e duplica o verificador que o
  Seiva já possui. Os JWTs mantêm a verificação stateless (ADR 0006); a introspection
  fica disponível para o caso opaco/de-referência.
- **Dobrar o authorization server dentro do `seiva-identity`.** Rejeitado: a emissão
  tem um registo de clientes, um UI de consentimento, custódia de chaves de assinatura
  e um store de revogação — um recurso e um modelo de falha diferentes da verificação
  stateless. Um crate separado mantém o `seiva-identity` network-free e testável.

## Consequências

- **Performance.** Os access tokens JWT são verificados sem estado pelo
  `seiva-identity` (sem round-trip por pedido); o JWKS próprio do server é cacheável
  com um `kid` estável; a introspection fica reservada aos tokens opacos. O crate não
  liga runtime e é zero-cost quando um serviço nada emite.
- **Segurança.** O PKCE é **obrigatório** (S256) em cada troca de code; os
  redirect-URIs são por **correspondência exact-string**; `state`/`nonce` vinculam o
  round-trip; os clientes confidenciais autenticam com um secret em hash; os
  authorization codes são de curta duração, de uso único e vinculados ao challenge;
  os scopes são allowlists **deny-by-default** (ADR 0005/0019); não existe grant
  implicit/password. Fail-closed de ponta a ponta.
- **Resiliência.** A revogação e a introspection dão ao operador um botão de corte; a
  rotação da chave de assinatura (`kid` actual+anterior) torna uma troca de chave numa
  operação sem downtime; a rotação de refresh e a detecção de reutilização são
  herdadas do ADR 0035; um cliente comprometido fica contido aos seus próprios scopes
  e redirect-URIs.
- O delonix-meet deixa de improvisar a emissão de tokens para os seus clientes MCP e
  web e monta o seu catálogo de scopes e o texto de consentimento como política por
  cima; o authorization server, construído uma vez e revisto uma vez, torna-se a raiz
  de confiança que tanto o fluxo SSO do ADR 0040 como a superfície MCP consomem.
- **Não** substitui um IdP enterprise completo (sem UI de federação de utilizadores,
  sem directório embutido): este é o authorization server de 1ª parte para os
  clientes próprios de uma plataforma Seiva, com o login OIDC como relying-party
  (ADR 0040) a cobrir a identidade externa.

# ADR 0047 — TLS mútuo no `seiva-http`: um certificado de cliente verificado é mais uma credencial do mesmo modelo de identidade

- Status: Aceite
- Data: 2026-11-26
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade e autenticação), ADR 0012 (sondas de saúde e limites cloud-native), ADR 0026 (controlo de carga por rota)

## Contexto

Alguns serviços Seiva são chamados sobretudo por **máquinas**: agentes de posto,
gateways de site, outros serviços. Para elas, um segredo bearer é uma má
credencial. Fica em disco, um proxy de intercepção TLS consegue lê-lo e vale até
alguém se lembrar de o rodar. A resposta habitual é o **TLS mútuo**: cada máquina
tem uma chave privada que gerou ela própria (num TPM, num keychain ou num ficheiro
`0600`) e um certificado de curta duração emitido pela CA da própria instalação,
e prova a posse da chave durante o handshake.

Até agora o Seiva só servia HTTP simples (`seiva_server::serve_router` sobre um
`TcpListener`); o TLS acabava num proxy reverso. Um projecto que quisesse mTLS
tinha duas más opções:

- **Terminar o mTLS no proxy e reencaminhar o certificado num cabeçalho**
  (`X-Client-Cert`). A aplicação passa a confiar num cabeçalho. Basta um proxy
  mal configurado, ou uma ligação directa que contorne o proxy, para qualquer
  cliente poder alegar qualquer certificado.
- **Correr um segundo listener próprio** ao lado do router Seiva, com
  autenticação própria. É um segundo modelo de identidade, fora do ADR 0006: o
  guard de permissões, o `Scoped` e a auditoria deixariam de ver um único
  `Principal`.

As forças:

- **O certificado verifica-se onde a chave é provada**, no handshake, e só
  contra a CA da instalação. Nunca se usam as raízes do sistema, para que nem uma
  CA pública nem a CA de um proxy de intercepção consigam emitir um cliente
  aceite.
- **Um só modelo de identidade.** Um certificado verificado tem de acabar no
  mesmo `Principal` que um JWT ou uma API key produzem, para que `#[requires]`,
  `Scoped` e a auditoria funcionem sem mudanças.
- **A revogação tem de ser rápida.** Uma CRL verificada no handshake é
  necessária, mas grosseira. A aplicação também guarda o seu próprio registo dos
  dispositivos enrolados, e consultá-lo em cada pedido faz a revogação valer a
  partir do pedido seguinte, e não só da ligação seguinte.
- **Pessoas e máquinas podem partilhar a mesma porta.** Um utilizador da UI com
  bearer OIDC e um agente com certificado podem bater no mesmo listener.
- **O handshake é superfície de ataque.** Um cliente que abre TCP e nunca acaba o
  handshake não pode parar o ciclo de accept nem reter memória para sempre.
- **Opcional e sem DORM.** Um serviço atrás de um proxy que termina TLS não liga
  nada disto.

## Decisão

Acrescentar TLS mútuo ao `seiva-http` atrás de uma **funcionalidade opcional
`mtls`**, com uma contraparte fina no `seiva-server` (funcionalidade `mtls`).

**No `seiva-http` (`seiva_http::mtls`):**

- **`MtlsConfig`**, construído a partir de bytes PEM: a cadeia e a chave do
  servidor, a **CA de clientes** (as únicas âncoras de confiança), **CRLs**
  opcionais (com uma opção para verificar só a folha, para uma PKI cuja raiz
  offline não publica CRL) e `ClientAuth::Required` (por omissão) ou
  `ClientAuth::Optional`. É **só TLS 1.3**, com o fornecedor `ring` do rustls e
  ALPN `h2` + `http/1.1`. Os dois protocolos são servidos: a feature `mtls`
  activa o `http2` do axum, para que um cliente que negoceie `h2` (o reqwest
  fá-lo) receba HTTP/2 e não uma ligação fechada (corrigido na 0.2.2). O `Debug`
  nunca mostra material de chave.
- **`MtlsListener`**, um `axum::serve::Listener`. Os accepts TCP correm numa
  tarefa de fundo. Cada handshake corre na sua própria tarefa, com **timeout**
  (10 s por omissão), e há no máximo **N handshakes** (1024 por omissão) em
  curso. O lugar é reservado *antes* do `accept`, para a contrapressão chegar à
  fila de accept do kernel. Só handshakes concluídos chegam ao `axum::serve`.
- **`mtls_make_service(router)`**, o serviço por ligação. Põe em cada pedido o
  `ConnectInfo<SocketAddr>`, para o rate limiter continuar a usar o IP do
  cliente, e o **`PeerCertificate`** da ligação: `Verified(ClientCertificate)` ou
  `Unreadable`. Qualquer valor desse tipo que já venha no pedido é substituído ou
  removido; um pedido nunca o consegue forjar.
- **`ClientCertificate`**: a folha verificada, já interpretada. Expõe o CN do
  sujeito, as SANs DNS e URI, o número de série, o `not_after`, a impressão
  digital SHA-256 e o DER. Só é construído a partir de uma cadeia que o rustls já
  verificou.
- **`CertificateAuthenticator`**: o mapeamento da aplicação de um
  `ClientCertificate` para um `Principal`. Qualquer
  `Fn(&ClientCertificate) -> Result<Principal, AuthError>` serve.

**No middleware `authn` (ADR 0006):** `AuthConfig::with_client_certificates`
acrescenta uma terceira fonte de credenciais, consultada **primeiro**:

- um certificado `Verified` vai ao authenticator, que devolve o `Principal` ou
  `401`. Corre no pool bloqueante, como a verificação do JWT, porque pode
  consultar um repositório;
- um certificado `Unreadable` é recusado com `401`; o middleware não adivinha;
- um certificado verificado **e** um cabeçalho `Authorization` ou `X-API-Key` no
  mesmo pedido é ambíguo, por isso é recusado com `401`: **um pedido, uma
  identidade**;
- sem certificado (só possível com `ClientAuth::Optional`), o pedido segue pelos
  caminhos `X-API-Key` → bearer → anónimo, sem mudanças.

**No `seiva-server`:** `serve_router_mtls(addr, router, health, &mtls, shutdown)`
é o `serve_router` sobre um `MtlsListener`, com a mesma verificação de colisão de
rotas e o mesmo ciclo de vida Starting → Ready → Draining. `MtlsFiles` lê a
configuração 12-factor (`SEIVA_TLS_CERT_FILE`, `SEIVA_TLS_KEY_FILE`,
`SEIVA_TLS_CLIENT_CA_FILE`, `SEIVA_TLS_CLIENT_AUTH`, `SEIVA_TLS_CRL_FILE`).
Certificado, chave e CA vêm juntos ou não vêm; um conjunto parcial impede o
arranque. Um ficheiro que não se consegue ler é o erro de arranque
`ServeError::Tls`.

## Alternativas consideradas

- **Certificado num cabeçalho vindo do proxy.** Rejeitada: a identidade
  assentaria no caminho de rede, não na criptografia. Continua possível para uma
  instalação que a queira, como um `Authenticator` próprio, mas não é o mTLS do
  framework.
- **Um crate `seiva-mtls` à parte, com listener e middleware próprios.**
  Rejeitada: um segundo caminho de autenticação fora do `authn` é exactamente a
  deriva que o ADR 0006 existe para evitar. O listener vive junto do transporte
  que serve, atrás de uma funcionalidade.
- **Mapear o certificado para um `Principal` uma vez por ligação.** Rejeitada:
  uma ligação HTTP/2 longa manteria autenticado um dispositivo revogado até
  voltar a ligar. Mapear em cada pedido custa uma consulta e torna a revogação
  imediata. A interpretação do certificado corre uma vez por ligação.
- **`axum-server` com o seu acceptor rustls.** Rejeitada: traz um segundo ciclo
  de serve ao lado do `axum::serve` e continua a precisar de código próprio para
  levar o certificado ao pedido. `axum::serve::Listener` com um make-service é
  mais pequeno e mantém a paragem graciosa que o Seiva já tem.
- **Aceitar TLS 1.2.** Rejeitada para este listener: os clientes são agentes e
  serviços que a própria instalação controla, por isso não há cliente antigo a
  manter. O TLS 1.3 elimina a superfície de renegociação e de cipher suites.

## Consequências

- Um projecto liga `seiva-server/mtls`, define as `SEIVA_TLS_*`, acrescenta
  `AuthConfig::with_client_certificates(...)` e chama `serve_router_mtls`. As
  facades, o `#[requires]`, o `Scoped` e a auditoria ficam iguais.
- O build por omissão não muda e não liga nenhuma pilha TLS. Com a
  funcionalidade, as dependências novas são `rustls` (ring), `tokio-rustls`,
  `x509-parser` e `tower-service`. Todas têm licença MIT/Apache e passam o
  `cargo deny`.
- O job `check` do CI corre explicitamente os testes `mtls`
  (`cargo test -p seiva-http -p seiva-server --features …/mtls`), porque o
  `cargo test --workspace` não os compila.
- A PKI em si **não** faz parte do Seiva: emitir, rodar e revogar certificados,
  e o fluxo de enrolamento por CSR, são da aplicação. O Seiva verifica e mapeia.
- Ainda não há recarga a quente do certificado nem da CRL: mudá-los obriga a
  reiniciar. Um `MtlsConfig` recarregável pode vir depois sem mudar o contrato do
  `CertificateAuthenticator`.

## Relacionadas

- ADR 0006: identidade e autenticação (o `Principal`, a ordem do `authn`).
- ADR 0012: sondas de saúde (Ready/Draining à volta do listener).
- ADR 0026: controlo de carga por rota (o limite de handshakes é a contraparte ao
  nível da ligação).

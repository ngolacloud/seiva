# ADR 0039 — `seiva-scim`: provisioning de utilizadores e grupos SCIM 2.0

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade e autenticação), ADR 0009 (registo de admin por módulo)

## Contexto

Um cliente enterprise não cria utilizadores na tua plataforma à mão. O identity
provider dele — Okta, Entra ID, Google Workspace — é a fonte de verdade, e espera
*empurrar* o directório: provisionar um utilizador quando um empregado entra,
dar-lhe patch quando muda de equipa, e **desprovisioná-lo** no momento em que sai.
O protocolo para esse push é o **SCIM 2.0** (System for Cross-domain Identity
Management, RFC 7643/7644). Uma plataforma sem endpoint SCIM não pode ser vendida a
uma organização que corre automação centralizada de joiner/mover/leaver — regride
para administração manual de contas, que é precisamente o que o cliente paga para
evitar.

O delonix-meet já carrega isto: uma superfície SCIM a mapear o directório de um IdP
sobre as suas orgs/branches/seats. É mecanismo puro soldado ao directório do
produto. Toda a plataforma que integra enterprises precisa do mesmo CRUD de
`/Users` e `/Groups`, da mesma filtragem e paginação, do mesmo sync idempotente —
enquanto *aquilo a que um utilizador mapeia* (um seat, um branch, um papel) é
política de domínio. Divisão de upstreaming clássica.

As forças:

- **O IdP é quem escreve; nós somos o sumidouro.** O provisioning é um fluxo de
  entrada, com o servidor como autoridade: o IdP faz `POST`/`PATCH`, nós
  reconciliamos. O nosso store de identidade (ADR 0006) é o alvo; a camada SCIM é um
  adapter à frente dele, não um segundo directório.
- **Idempotente por `externalId`.** Um IdP faz retry. O mesmo utilizador empurrado
  duas vezes não pode criar duas contas. O `externalId` (a chave estável do IdP) é a
  âncora de idempotência: um create para um `externalId` conhecido é um
  no-op-ou-update, nunca um duplicado. O sync inteiro tem de ser **reconciliável** —
  voltar a correr converge.
- **O desprovisioning é uma operação de 1ª classe, não um `DELETE`.** Quem sai é
  desactivado/soft-deleted (`active: false`), preservando o histórico de auditoria e
  a integridade referencial, não hard-deleted por baixo dos dados que o referenciam.
  A flag `active` do IdP comanda o estado da conta.
- **Tenant-scoped e fail-closed.** Um token SCIM pertence a exactamente um tenant;
  cada operação de `/Users` e `/Groups` é scoped a esse tenant sem caminho fail-open
  (ADR 0019). Um IdP do tenant A nunca pode ler, dar patch ou desprovisionar o
  directório do tenant B.
- **Minimização de PII.** O directório carrega nomes, emails, números de empregado.
  Só os atributos que o produto mapeia são guardados; os atributos SCIM não mapeados
  não são acumulados em silêncio, e secrets/tokens nunca aterram nos logs.

## Decisão

Adicionar um novo membro do workspace **`seiva-scim`** — uma superfície de
provisioning SCIM 2.0, sem camada de dados, atrás de feature, a mapear sobre a
identidade/directório do ADR 0006.

- **`/Users` e `/Groups`** — os endpoints de recurso SCIM 2.0: `POST` (criar),
  `GET` (ler/por-id), `PUT` (substituir), **`PATCH`** (actualização parcial RFC
  7644, a operação em que os IdPs se apoiam) e `DELETE` (encaminhado para desactivar,
  não hard-delete). As respostas carregam o envelope SCIM `meta`/`schemas` e o
  emparelhamento `id`↔`externalId`.
- **Filtragem** — a query `filter` do SCIM (ex.: `userName eq "a@b.com"`,
  `externalId eq "..."`), parseada para um conjunto de atributos **limitado e em
  allowlist** e empurrada para o store como um lookup indexado — nunca uma expressão
  arbitrária contra a base de dados.
- **Paginação** — cursoramento `startIndex`/`count` com um tamanho de página limitado
  pelo servidor, para que um IdP a enumerar um directório grande não possa pedir uma
  varredura sem limite.
- **Bulk** — o endpoint SCIM `/Bulk` para operações em lote, cada entrada carregando
  a sua própria idempotência para que uma falha parcial seja por-entrada, não
  tudo-ou-nada.
- **Sync idempotente por `externalId`** — um port `DirectorySink` mapeia um recurso
  SCIM para o utilizador/grupo do produto via `externalId`: um mapeamento existente
  actualiza, um ausente cria, e um re-push converge. O port é a costura onde o produto
  declara o **mapeamento org↔directório** e quaisquer **atributos custom** (um seat,
  um branch, um claim de papel).
- **Desprovisioning** — `active: false` (via `PATCH` ou `DELETE`) desactiva a conta
  através do store de identidade, preservando a linha e o seu rasto de auditoria.
- **Auth e scope** — um bearer token emitido ao IdP (via ADR 0038 ou uma chave de
  provisioning) autentica cada pedido; o tenant do token dá scope a cada operação,
  fail-closed.

O `tokio` entra com `default-features = false`; o `DirectorySink` é um port com um
duplo em memória para testes e um adapter suportado por DORM atrás da feature
`dorm`. Os recursos provisionados registam-se como `AdminResource`s (ADR 0009) para
que um operador veja o directório sincronizado na consola.

## Alternativas consideradas

- **Uma API de provisioning à medida por produto.** Rejeitado: os IdPs falam SCIM,
  não o teu dialecto REST. Um endpoint não-SCIM significa que cada cliente enterprise
  escreve um conector custom — um bloqueador de vendas e um imposto de manutenção. O
  SCIM é o contrato de interop que os compradores já correm.
- **Hard-delete no `DELETE`.** Rejeitado: a linha de utilizador de quem sai é
  referenciada por reuniões, entradas de auditoria e registos de propriedade. A
  desactivação preserva a integridade e o histórico; um hard delete deixa dados
  órfãos e apaga o rasto de auditoria — o oposto do que um enterprise orientado a
  conformidade quer.
- **Confiar no `id` do IdP como chave.** Rejeitado: o nosso `id` e o do IdP são
  namespaces diferentes. O `externalId` é a chave estável do IdP *por desenho*;
  ancorar a idempotência nele é o que a spec pretende e o que faz o re-sync convergir.
- **Dobrar o SCIM dentro do `seiva-identity`.** Rejeitado: o `seiva-identity` é um
  verificador sem transporte (ADR 0006); o SCIM é uma superfície HTTP de
  provisioning de entrada com o seu próprio envelope de esquema, gramática de filtro e
  semântica de bulk. Um crate separado mantém o verificador network-free e deixa o
  provisioning evoluir independentemente.

## Consequências

- **Performance.** Os filtros compilam para lookups de atributos **indexados** sobre
  uma allowlist limitada (sem query arbitrária); a paginação é limitada pelo servidor;
  o `/Bulk` agrupa a importação inicial de um IdP. O crate não liga runtime e é
  zero-cost para uma plataforma que não oferece provisioning.
- **Segurança.** Cada pedido é autenticado pelo bearer token do IdP e
  **tenant-scoped fail-closed** (ADR 0019) — o provisioning cross-tenant é
  impossível; só os atributos mapeados são guardados (**minimização de PII**); os
  secrets nunca entram nos logs. Um IdP de um tenant não pode tocar no directório de
  outro.
- **Resiliência.** A idempotência por `externalId` significa que um push repetido ou
  reenviado nunca duplica uma conta; o sync inteiro é **reconciliável** — voltar a
  correr converge para o estado do IdP; o desprovisioning é uma desactivação
  reversível que preserva o histórico de auditoria e a integridade referencial.
- O delonix-meet deixa de manter uma superfície SCIM construída à mão e mantém apenas
  o seu **mapeamento org↔directório** (que atributo SCIM se torna um seat/branch/papel)
  como política no `DirectorySink`; toda a futura plataforma Seiva ganha automação de
  joiner/mover/leaver enterprise de borla.
- **Não** torna o Seiva um identity provider: o SCIM é o sumidouro *de entrada* que
  deixa um IdP externo ser dono do directório, complementando o fluxo de login OIDC
  (ADR 0040) no lado da autenticação.

# ADR 0008 — A consola de administração é uma extensão, não parte da base

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0003, ADR 0005, ADR 0006, ADR 0007

## Contexto

O Seiva ganhou uma consola de administração como item de roadmap P1-5: `admin.rs`
dentro de `seiva-http`, servido em `GET /admin` por `framework_routes`, de modo
que **cada** backend Seiva — o esqueleto sem módulos incluído — entregava o
HTML/CSS/JS da consola e respondia `200` em `/admin`. A página é uma UI de
operações auto-contida e sem CDN que lê o próprio `/openapi.json` do serviço (ADR
0007) e renderiza um formulário de ensaio por operação de escrita, exercido com
um token bearer.

Três problemas com essa colocação:

1. **Fronteira.** A base do framework destina-se a ser uma peça de construção
   HTTP publicável sem opinião sobre a UX do operador. Embeber uma UI de
   administração em `seiva-http` significa que um backend que não queira
   superfície de administração mesmo assim a compila, entrega e serve — os
   assets da consola estão no binário e a rota está viva. Não há forma de obter um
   backend com *nenhum* código de administração; o `cargo tree` mostra-o sempre
   porque *é* a crate de transporte.

2. **Postura de segurança.** O `Content-Security-Policy` da consola usava
   `script-src 'unsafe-inline'` e `style-src 'unsafe-inline'` — os próprios
   cabeçalhos de segurança por omissão da base do framework são estritos, mas a
   rota de administração alargava-os para a página toda. `unsafe-inline` derrota o
   propósito de um CSP: qualquer `<script>` injectado corre.

3. **Honestidade.** Entregar `/admin` na base lê-se como "o Seiva tem um admin
   auto-CRUD completo de origem". Não tem: este primeiro corte conduz as
   *operações* do módulo que o contrato declara, não CRUD genérico de linhas de
   modelo, e não tem registo por módulo, mascaramento de campos do lado do
   servidor nem autorização por operação própria.

As forças: a consola é genuinamente útil e já gerada puramente a partir do
contrato vivo (sem código por módulo), por isso deve continuar a funcionar; mas a
base tem de conseguir construir, arrancar e servir com **nenhum** código
administrativo; e a dependência tem de correr num só sentido — uma extensão pode
depender da API pública da base, nunca o contrário, e a base não pode ganhar uma
feature `default = ["admin"]` que silenciosamente as re-acople.

## Decisão

**Extrair a consola para uma crate separada e opcional `seiva-console-admin`, uma
*extensão* da base Seiva. A base (`seiva-http`) não leva código de administração
e responde `404` para `/admin`. Uma raiz de composição opta explicitamente pela
consola chamando `seiva_console_admin::mount(router)` e recompilando; nada na
base depende da extensão.**

Concretamente:

- `seiva-http` perde `mod admin`, as re-exportações `admin_handler`/`ADMIN_HTML`
  e a rota `/admin` em `framework_routes`. A sua superfície é agora exactamente
  health, `/openapi.json` e `/docs`. Um teste de regressão afirma que `/admin` é
  `404`.
- `seiva-console-admin` é um membro do workspace (para que a CI o construa e
  teste) que depende apenas de `axum` (o tipo de transporte público da base) mais
  `sha2`/`base64` para os hashes do CSP. **Nenhuma crate da base depende dela**,
  por isso um backend construído sem ela não tem assets de administração nem rota
  `/admin` — demonstrável com `cargo tree`.
- A crate expõe `admin_router() -> Router`, `mount(Router) -> Router` e
  `ADMIN_PATH`. `mount` é a adesão explícita e auditável; um segundo `mount` entra
  em panic em tempo de construção do router (o axum rejeita uma rota duplicada),
  por isso um duplo-mount é um erro de arranque, nunca um silencioso.
- **Endurecimento do CSP.** O `<style>` e o `<script>` inline são fixados pelo seu
  hash SHA-256 (`script-src 'sha256-…'; style-src 'sha256-…'`), computado em
  runtime a partir dos bytes exactos servidos, de modo que a política nunca possa
  derivar do payload. O `unsafe-inline` desapareceu. A política acrescenta
  `base-uri 'none'`, `form-action 'none'` e mantém `default-src 'none'`,
  `connect-src 'self'`, `frame-ancestors 'none'` e `img-src 'self' data:` — sem
  origem remota em lado nenhum.

### Fora desta ADR (incrementos posteriores)

Esta ADR move a consola existente e endurece o seu CSP. **Não** acrescenta:

- Registo por módulo (`seiva module new --admin`, um `AdminRegistry`, formulários
  tipados, lista/filtro/pesquisa/ordenação/paginação, acções personalizadas) —
  Fase 3.
- Autorização do lado do servidor *pela consola*
  (por-operação/objecto/inquilino/campo), para além dos guards que o
  `#[seiva::controller]` de cada módulo já impõe nas rotas que a consola chama —
  Fase 3/5.
- Um protocolo de integração DX `dx add seiva-console-admin` — Fase 3.

Esses são rastreados em separado para que esta extracção possa aterrar como um
passo completo e verificável.

## Consequências

**Positivo.**

- Um backend sem administração constrói, arranca e serve sem código
  administrativo; `/admin` é `404` e o `cargo tree` não mostra
  `seiva-console-admin`.
- A base do framework é de novo uma peça de construção de transporte limpa; a
  consola evolui à sua própria cadência sem tocar em `seiva-http`.
- A página de administração já não enfraquece o seu CSP com `unsafe-inline`.
- A aresta de dependência é de sentido único e explícita: a extensão depende da
  API pública da base; a base nunca depende da extensão.

**Negativo / custo.**

- Projectos existentes que dependiam de `/admin` estar presente por omissão têm
  de acrescentar a crate e uma chamada `mount(..)` de uma linha (documentado no
  guia de migração).
- Os assets da consola são duplicados para fora do histórico de `seiva-http` para
  a nova crate (a mudança preserva o código; o `git` regista a remoção +
  adição).
- Uma raiz de composição tem de recompilar para ganhar a consola — por design:
  uma dependência declarada só por si não monta rotas (a ADR segue a regra de
  "composição explícita" da ADR 0003).

## Alternativas consideradas

- **Feature flag `seiva-http/admin` (por omissão desligada).** Rejeitada: mantém
  o código e os assets de administração dentro da crate base (código ainda
  entregue, acoplamento de manutenção ainda) e convida a uma regressão `default =
  ["admin"]`. Uma crate de extensão torna a fronteira física.
- **Manter `/admin` na base, só endurecer o CSP.** Rejeitada: corrige o CSP mas
  não a fronteira nem o problema de honestidade; um backend sem administração
  continua impossível.
- **Nonce por pedido em vez de hashes.** Um nonce precisa de um CSPRNG por
  resposta e injecção no template; para uma página totalmente estática, um hash
  sobre os bytes servidos é mais simples e não tem dependência de aleatoriedade em
  runtime. Rever se a página ganhar conteúdo inline específico do pedido.

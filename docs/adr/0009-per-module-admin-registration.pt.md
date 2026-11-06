# ADR 0009 — Registo de administração por módulo e o portão de autorização

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0005, ADR 0006, ADR 0007, ADR 0008

## Contexto

A ADR 0008 extraiu a consola de administração para a extensão opcional
`seiva-console-admin`. Esse primeiro corte renderizava **cada** operação em
`/openapi.json`: estar no contrato bastava para aparecer na consola. Dois
problemas:

1. **Exposição não é autorização (OWASP API9/API1).** O linker a incluir uma
   fachada não a deve pôr numa UI de operador. Um módulo deve decidir,
   explicitamente, se é administrável, e sob que permissões.
2. **Sem metadados detidos pelo módulo.** Rótulos, colunas de lista, campos
   pesquisáveis/ordenáveis e permissões por operação não têm onde viver.
   Frameworks maduros deixam um módulo declarar isto num manifesto de
   administração dedicado; o Seiva não tinha equivalente.

Restrições do brief do dono:

- Um módulo que adira não pode tomar dependência da crate da **UI** de
  administração (`seiva-console-admin`): desactivar a extensão não pode deixar
  dependência obrigatória nas camadas domain/application de um módulo.
  "Registar-e-esquecer" não pode acoplar o módulo à consola.
- A consola só deve mostrar um recurso quando ele está **registado** *e* o
  chamador está **autorizado** a vê-lo. Um `tenant_id` vindo de um corpo de
  pedido nunca pode decidir o acesso; o servidor, de posse do `Principal`
  autenticado, decide.
- Um registo duplicado (dois módulos a reclamar um `(namespace, resource)`) tem
  de falhar ruidosamente, no build ou no arranque, não ensombrar um ao outro
  silenciosamente.

## Decisão

**Um módulo regista um descriptor `AdminResource` no inventário em tempo de
ligação da crate do framework via `delonix_seiva::register_admin!`. A consola lê
o registo, filtra-o pelas permissões do `Principal` do chamador, e mostra apenas
recursos registados *e* autorizados. O tipo do descriptor vive em `delonix-seiva`,
não na crate da consola.**

- `delonix_seiva::admin` é dona de `AdminResource` (id estável `(namespace,
  resource)`, `label`, `facade`/`version` de suporte,
  `list_fields`/`search_fields`/`filters`/`ordering`/`page_size`, e
  `AdminPermissions { view, create, change, delete }`) mais
  `registered_admin_resources()` e `admin_registry_conflicts()`. Reutiliza o
  exacto mecanismo de `inventory` que as fachadas usam (ADR 0001/0007), por isso
  não existe lista mantida à mão.
- A camada **interface** de um módulo chama `register_admin!(AdminResource {
  … })`. Como o descriptor está em `delonix-seiva` — uma crate de que a camada
  interface já depende para `#[controller]`/`#[facade]` — registar não acrescenta
  **nenhuma** dependência de `seiva-console-admin`. Com a consola desmontada, o
  registo é dados inertes em tempo de ligação (sem custo em runtime, sem
  comportamento). Um módulo que não se registe é **privado à consola**.
- `seiva-console-admin` serve `GET /admin/registry.json`, construído a partir de
  `registered_admin_resources()` filtrado do lado do servidor: um recurso só
  aparece quando a sua permissão `view` é não-vazia **e** o `Principal` do pedido
  (do middleware `authn`, por omissão anónimo) a detém. A permissão de cada
  operação é resolvida para uma flag `can` para que a UI esconda o que o chamador
  não pode fazer — enquanto o guard do `#[controller]` do módulo se mantém a
  autoridade em cada chamada (defesa em profundidade). A UI da consola renderiza
  apenas os recursos que o registo devolve, emparelhando cada um com as suas
  operações de `/openapi.json` por segmento de recurso.
- **Detecção de conflitos.** `admin_registry_conflicts()` devolve os ids
  duplicados; o endpoint do registo devolve `500` (listando-os) em vez de servir
  um conjunto ensombrado, e `seiva_console_admin::check()` deixa uma raiz de
  composição transformar um duplicado num erro de arranque.

A permissão em que a consola faz o portão é a *mesma string* que o método da
fachada declara em `#[requires(..)]` (ADR 0006) — uma fonte única de verdade para
a autorização.

### Fora desta ADR (incrementos posteriores)

- `seiva module new --admin` a gerar a chamada `register_admin!` — Fase 3b.
- Um protocolo de integração DX `dx add seiva-console-admin` + o equivalente
  Cargo — Fase 3c.
- CRUD genérico de linhas de modelo (lista/detalhe/edição de linhas de dados),
  mascaramento ao nível do campo e leituras de dados com âmbito de inquilino na
  consola — mais tarde, com a administração da camada de dados.

## Consequências

**Positivo.**

- A exposição é agora uma decisão explícita e por módulo; a consola nunca
  apresenta um recurso só porque está no contrato.
- A autorização é imposta do lado do servidor contra o `Principal` autenticado,
  com as mesmas strings de permissão que os guards da API usam.
- Os módulos declaram metadados de administração sem depender da UI de
  administração; a extensão mantém-se opcional e de sentido único.
- O duplo-registo falha ruidosamente.

**Negativo / custo.**

- Um módulo tem de acrescentar um pequeno bloco `register_admin!` para aparecer
  (até 3b o gerar). Este é o compromisso "explícito, auditável" pretendido.
- O registo é em tempo de ligação: uma ferramenta que não liga um módulo não vê
  os seus recursos (a mesma propriedade que as fachadas têm). O binário de
  runtime que monta os módulos vê o conjunto completo, que é o que hospeda a
  consola.

## Alternativas consideradas

- **Tipo do descriptor em `seiva-console-admin`.** Rejeitada: um módulo que se
  registe passaria então a depender da crate da UI de administração, violando
  "desactivar a extensão não deixa dependência obrigatória".
- **Derivar a visibilidade de `/openapi.json` + uma convenção de permissão.**
  Rejeitada: isso mantém "no contrato ⇒ na consola" e não oferece lugar para
  rótulos, colunas ou permissões por operação.
- **Um `AdminRegistry` em runtime construído por uma chamada
  `register_admin(&mut registry)` na raiz de composição.** Viável, mas reintroduz
  uma lista central mantida à mão; o inventário em tempo de ligação corresponde a
  como as fachadas já se auto-registam e não precisa de ligação central.

## Adenda (2026-11-05) — o backend de administração da camada de dados (CRUD de linhas)

A primeira fatia do seguimento "CRUD de linhas de modelo" está construída e
provada, mantendo a regra acima (a consola conduz **operações de contrato**,
nunca acesso directo a tabelas, para que os invariantes de domínio fiquem no
módulo):

- `apps/admin-data-e2e` é um módulo CRUD compacto suportado por DORM — uma fachada
  `Catalog` com as cinco operações (`create`/`get`/`list`/`update`/`delete`)
  sobre uma tabela PostgreSQL real via `DormUnitOfWork` — que regista um
  `AdminResource` (`catalog.read/create/change/delete`) e monta a consola por trás
  de um autenticador de token. O acoplamento DORM torna-a uma crate excluída
  (construída pelos jobs de CI com token, auditada por `deny.excluded.toml`).
- Um teste vivo (`live_catalog_full_crud_round_trip`, condicionado a
  `SEIVA_DORM_TEST_DSN`) prova que create → get → list (ordenado) → update →
  delete persistem cada um através do DORM contra PostgreSQL real.

A **UI** de CRUD de linhas da consola está também construída: um recurso que expõe
uma operação `list` renderiza uma **tabela** de administração (colunas de
`list_fields`) com um formulário de criação, **edição** por linha (um formulário
inline a partir do schema `update`, pré-preenchido) e **remoção** (confirmar →
`delete`), tudo através das operações de contrato do módulo. Um E2E de browser
(`e2e/admin/catalog.mjs`) conduz o ciclo completo contra o módulo `Catalog` vivo
sobre PostgreSQL num Chrome headless real — create → aparece na lista → editar →
remover → desapareceu — e apanhou um bug real de correspondência de recurso
singular/plural.

**O âmbito de inquilino e o mascaramento de campos também estão feitos**, do lado
do servidor via uma costura `&Ctx` completada: o `#[seiva::controller]` agora
**injecta** um `seiva_http::Ctx` (que leva o `Principal` autenticado) em qualquer
método que receba `&Ctx`. As operações `Catalog` delimitam cada query por
`ctx.tenant()` (um chamador nunca vê nem toca nas linhas de outro inquilino) e
mascaram o campo interno `cost` a menos que o chamador detenha
`catalog.cost.view`. Provado vivo: dois inquilinos ficam isolados (globex não vê
nenhuma das linhas de acme e uma remoção entre inquilinos é um no-op), e um
chamador sem `cost.view` obtém `cost = None` na mesma linha que o dono vê por
inteiro.

**A paginação, a pesquisa e a ordenação também estão ligadas** (2026-11-05). A
operação `list` recebe um `ListQuery` opcional (lido da query string via o
`Query<T>` da macro do controller), e a consola renderiza uma caixa de pesquisa,
cabeçalhos de coluna ordenáveis clicáveis e paginação ant./seg. — tudo conduzido
a partir das allow-lists `search_fields`/`ordering`/`page_size` declaradas no
registo. O servidor valida a coluna de ordenação contra uma allow-list fixa e
parametriza o termo de pesquisa, de modo que um cliente nunca pode injectar um
nome de coluna ou uma página sem limite.

**A concorrência optimista também está feita.** As linhas levam um `version`;
`UpdateItem` ecoa a versão que o chamador leu por último, a actualização só se
aplica se ainda corresponder (incrementando-a), e uma edição desactualizada é um
`409` — a consola carrega a versão só-de-leitura no formulário de edição e expõe
o conflito. Provado por `live_catalog_list_search_order_paginate`, a asserção de
conflito-de-versão em `live_catalog_crud_tenant_scoped_and_masked`, e o E2E de
browser (`e2e/admin/catalog.mjs`: a pesquisa filtra a tabela; um incremento
concorrente seguido de uma gravação desactualizada mostra o `409`).

Ainda pendente: nada nesta fatia — o trabalho restante da consola (acções em
massa, filtros mais ricos) é âmbito futuro, não uma lacuna aberta no brief.

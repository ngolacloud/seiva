# ADR 0044 — RLS nativo do Postgres como backstop fail-closed sob o scoping aplicacional

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0004 (o gateway de camada de dados DORM), ADR 0019 (autorização fail-closed), ADR 0030 (a camada de dados é plugável)

## Contexto

O Seiva impõe o isolamento de tenant **na aplicação**: um `Scope` é passado por
cada caso de uso e os adapters de repositório filtram por `tenant_id`. Isso está
correcto e é onde a política pertence — mas é uma *única* linha de defesa. Um método
de repositório que se esqueça do `WHERE tenant_id = $1`, uma query escrita à mão, um
adapter novo que não aplique o scope, e o tenant A lê as linhas do tenant B, em
silêncio. O scope aplicacional é apenas tão forte quanto a query mais descuidada do
código.

Uma base de dados que suporte **Row-Level Security** pode ser uma segunda linha
independente: mesmo uma query sem predicado de tenant devolve **zero linhas** a
menos que a sessão actual tenha declarado o seu tenant. O delonix-meet já corre este
padrão — um `tenant_tx` que emite `SET LOCAL app.tenant_id = …` e políticas `FORCE
ROW LEVEL SECURITY` em cada tabela de tenant — e chama-lhe um backstop fail-closed.
Este ADR eleva isso para o Seiva como uma **opção da camada de dados**,
explicitamente como *defense-in-depth sob* o scope aplicacional, nunca como
substituto dele.

As forças:

- **Backstop, não substituto.** O `Scope` aplicacional continua a ser o mecanismo
  primário e portável (funciona para os doubles in-memory e qualquer camada de
  dados, ADR 0030). O RLS é a *segunda* linha que apanha o que o código deixa
  passar. Remover o scope aplicacional e «confiar no RLS» é rejeitado: acoplaria a
  correcção a uma base de dados e tiraria a política do domínio.
- **Fail-closed por construção.** O valor todo é que um predicado *em falta* produz
  *nenhuma* linha. A política tem de ser `USING (tenant_id = current_setting(...))`
  para que, se o GUC de sessão estiver por definir, a comparação seja null e **nada**
  corresponda — deny-by-default (ADR 0019), na camada de armazenamento.
- **O owner não pode escapar.** O RLS simples é contornado pelo dono da tabela e por
  papéis `BYPASSRLS`. O `FORCE ROW LEVEL SECURITY` faz a política aplicar-se mesmo ao
  dono, para que um ORM a ligar-se como o papel dono continue restringido.
- **O scope tem de chegar ao GUC.** O adapter de dados tem de vincular o `Scope` do
  request a `SET LOCAL app.tenant_id`/`app.user_id` **dentro da transacção**, para
  que o setting viva exactamente o tempo da transacção e nunca vaze por uma conexão
  com pool. Esta é a costura que o framework tem de possuir.
- **Jobs sem-sessão precisam de um bypass auditado.** Um job de fundo (ADR 0034),
  uma migração, um relatório admin inter-tenant corre sem um `Scope` de request. Tem
  de usar um caminho elevado **explícito e auditado**, não um GUC acidentalmente por
  definir — o bypass é uma decisão deliberada e registada, não um buraco.

## Decisão

Adicionar **RLS nativo do Postgres como opção da camada de dados** — parte dos
adapters DORM/sqlx (ADR 0004/0030), desligada por omissão, activada por implantação
— especificada como um backstop por baixo do `Scope` aplicacional.

- **Vinculação de scope por transacção.** Cada transacção com scope abre com
  `SET LOCAL app.tenant_id = $tenant` (e `app.user_id` onde uma política o precise),
  derivado do `Scope` do request. O `SET LOCAL` prende o GUC à transacção, para que
  uma conexão com pool devolvida não carregue tenant residual.
- **Políticas geradas.** As migrações para tabelas com scope de tenant geram, por
  tabela: `ALTER TABLE t ENABLE ROW LEVEL SECURITY; ALTER TABLE t FORCE ROW LEVEL
  SECURITY;` e uma política `USING (tenant_id = current_setting('app.tenant_id')::uuid)`
  (com um `WITH CHECK` correspondente para escritas). Uma passagem de ferramental
  verifica que cada tabela com scope de tenant tem de facto a sua política — uma
  tabela sem ela é uma falha de build, não uma lacuna silenciosa.
- **Semântica fail-closed.** Sem `app.tenant_id` definido,
  `current_setting('app.tenant_id', true)` é null, o predicado é null, e a query
  devolve zero linhas e nada escreve — o backstop é deny-by-default (ADR 0019).
- **Bypass explícito e auditado para trabalho sem-sessão.** Um helper
  `with_rls_bypass(reason)` corre num papel autorizado a definir um contexto
  inter-tenant (ou `BYPASSRLS`), e **cada** uso regista um registo de auditoria (ADR
  0043) com a razão e o chamador. Jobs, migrações e relatórios inter-tenant passam
  só por este caminho.
- **Opt-in por camada de dados.** O RLS é uma propriedade dos adapters apoiados em
  Postgres; os doubles in-memory e as camadas não-Postgres mantêm só-scope-de-app.
  Activar o RLS não altera código de aplicação nenhum — é uma escolha de implantação
  e migração (a plugabilidade do ADR 0030).

A política — *quais* tabelas têm scope de tenant e o modelo de tenancy — é do
produto; o mecanismo — a vinculação do GUC, as políticas geradas, o RLS forçado, o
bypass auditado — é do framework.

## Alternativas consideradas

- **Só scoping aplicacional (hoje).** Rejeitado como suficiente para implantações de
  alta garantia: está correcto mas tem uma só camada; uma query esquecida é uma fuga
  inter-tenant sem backstop. O RLS acrescenta uma segunda linha independente.
- **Só RLS, largar o scope de app.** Rejeitado: solda a correcção de tenant a uma
  base de dados, perde a portabilidade pelas camadas de dados plugáveis (ADR 0030) e
  pelos doubles in-memory, e tira a política do domínio para DDL. O defense-in-depth
  quer *ambos*, não uma troca.
- **RLS simples sem `FORCE`.** Rejeitado: o papel dono (que o ORM comummente usa)
  contorna o RLS não-forçado, pelo que o backstop não se aplicaria exactamente à
  conexão que dele precisa. O `FORCE ROW LEVEL SECURITY` é obrigatório.
- **Um schema ou base de dados por tenant.** Rejeitado como default: schema-por-tenant
  escala mal para muitos tenants e complica migrações e pooling; um único schema com
  uma coluna de tenant + RLS forçado é o backstop pragmático. (Continua a ser uma
  escolha legítima para alguns tenants muito grandes — ortogonal a este ADR.)

## Consequências

- **Performance.** O `current_setting` é uma leitura barata por transacção; o
  predicado `tenant_id` anda sobre índices existentes (cada tabela com scope de
  tenant indexa `tenant_id`), pelo que o RLS acrescenta um filtro de factor
  constante, não um scan; o `SET LOCAL` é uma instrução por transacção.
- **Segurança.** **Defense-in-depth** — o RLS apanha qualquer query que o scope
  aplicacional não restrinja; o `FORCE ROW LEVEL SECURITY` significa que **nem o dono
  da tabela** escapa; **fail-closed** — um GUC por definir produz zero linhas (ADR
  0019); a única forma de o ultrapassar é um bypass **explícito e auditado** (ADR
  0043) para trabalho sem-sessão.
- **Resiliência.** O backstop é **independente da correcção do código de
  aplicação**: um refactor futuro, um adapter novo, uma query escrita à mão não podem
  reintroduzir uma fuga inter-tenant enquanto o RLS estiver activo, pelo que o
  isolamento não degrada à medida que o código cresce.
- O padrão `tenant_tx`/`FORCE RLS` do delonix-meet torna-se uma opção do framework
  que qualquer implantação pode ligar, e toda a plataforma que lide com dados de
  múltiplos tenants ganha uma garantia de isolamento ao nível do armazenamento sob o
  seu scope aplicacional.

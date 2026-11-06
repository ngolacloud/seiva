# ADR 0014 — Auditoria da cadeia de fornecimento dos lockfiles excluídos e acoplados ao DORM

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0004 (gateway DORM)

## Contexto

O `cargo-deny` corre na CI sobre o workspace (por omissão e all-features) — mas o
workspace **exclui** as crates acopladas ao DORM (`seiva-data-dorm`,
`seiva-outbox-relay`, `seiva-rag-worker`, `seiva-mcp`, `apps/dorm-e2e`), por isso
as suas árvores de dependências — o grafo inteiro do DORM — **nunca são
auditadas**. O brief (§9) é explícito: "A análise default/all-features do
workspace principal não cobre automaticamente tudo"; estender a auditoria aos
lockfiles excluídos, registar excepções com expiração, e nunca baixar a política
para passar.

Auditar esses lockfiles fez de imediato aflorar três avisos que o workspace nunca
viu, todos na árvore do DORM:

- **RUSTSEC-2022-0004** (vulnerabilidade) e **RUSTSEC-2025-0025** (não mantido):
  `rustc-serialize`, puxado transitivamente por `eui48` (suporte de endereços MAC
  em `postgres-types`). O `eui48` usa-o apenas para o codec Decodable/Encodable de
  MAC, não para o parser vulnerável de JSON não confiável, por isso o overflow
  específico não é alcançado aqui.
- **RUSTSEC-2025-0134** (não mantido): `rustls-pemfile 2.x`, um invólucro fino
  puxado por `tokio-postgres-rustls` no caminho TLS do DORM. Sem vulnerabilidade
  conhecida.

A correcção para os três vive na árvore de dependências **do DORM**, não do
Seiva.

## Decisão

**Auditar o lockfile de cada crate excluída em busca de avisos RustSec na CI, com
um `deny.excluded.toml` dedicado cujas únicas flexibilizações são três excepções
temporizadas e fundamentadas que apontam o DORM como dono.**

- O job de CI `dorm-gateway` (com token, onde o DORM resolve) corre `cargo deny
  --config deny.excluded.toml check advisories` por crate excluída, depois de as
  construir/lintar/testar. Um aviso **novo** na árvore do DORM falha agora este
  portão em vez de ser invisível.
- O `deny.excluded.toml` não flexibiliza **nada** globalmente: é só `advisories`
  (`yanked = "deny"`), e a sua lista `ignore` nomeia exactamente os três avisos
  acima, cada um com uma razão, a crate que o puxa, o dono (DORM) e uma data de
  revisão (2026-02-28). O `deny.toml` do workspace fica intocado — o portão
  principal mantém toda a sua força; estas crates nunca entram no grafo do
  workspace de qualquer modo.

## Consequências

**Positivo.** A árvore do DORM é agora auditada em busca de avisos na CI; os três
problemas conhecidos são dívida rastreada (fundamentada, datada, com dono) em vez
de lacunas silenciosas; uma regressão (um novo CVE puxado pelo DORM) parte o
portão.

**Negativo / custo.** Três ignores a re-rever na data, e a largar à medida que o
DORM actualiza (`eui48`/`rustc-serialize`, `rustls-pemfile`). A auditoria só corre
quando `DORM_TOKEN` está configurado (como os outros jobs DORM); sem ele as crates
excluídas não são construídas nem auditadas de todo — inalterado face a antes, mas
vale a pena afirmá-lo.

## Alternativas consideradas

- **Acrescentar os ignores ao `deny.toml` do workspace.** Rejeitada: enlameia o
  portão principal com excepções para crates que o workspace nunca puxa; uma
  config só-advisories separada mantém a política principal imaculada e a dívida
  do DORM explícita.
- **Correr o `check` completo (licenças + bans) sobre as crates excluídas.**
  Adiada: a prioridade da §9 é o portão RustSec/CVE; a árvore grande do DORM
  precisaria da sua própria curadoria de allow-list de licenças, um seguimento
  separado. Avisos primeiro.
- **Ignorar os avisos sem uma config / silenciosamente.** Rejeitada
  categoricamente pela §9 ("Ausência de advisory não prova segurança", as
  excepções precisam de expiração).

# Auditoria de engenharia Rust — Seiva (2026-11)

- **Alvo:** `seiva` · ramo `main @ 985cfa1` · papel **Plataforma** (distributed systems)
- **Método:** 6 lanes read-only independentes (async, segurança, contratos/adopção,
  dados/eventos, observabilidade/ops, adopção-comunidade) + consolidação por quem não
  gerou as lanes. Arnês: `rust-engineering-audit`.
- **Âmbito:** ~24,5k LOC em 25 crates centrais + `delonix-seiva` (5,3k). Read-only —
  nenhum código alterado na fase de auditoria.

## Entregáveis

| # | Ficheiro | Conteúdo |
|---|---|---|
| 1 | [RUST_ENGINEERING_AUDIT.md](RUST_ENGINEERING_AUDIT.md) | Sumário executivo, metodologia, veredicto honesto, convergência de lanes |
| 2 | [COMPETENCY_MATRIX.md](COMPETENCY_MATRIX.md) | Competência × classe (Correct/Partial/Incorrect/Missing/N/A) |
| 3 | [ARCHITECTURE_GAPS.md](ARCHITECTURE_GAPS.md) | Lacunas de arquitectura: eventos, contratos/SemVer, tenant-scope |
| 4 | [PERFORMANCE_REPORT.md](PERFORMANCE_REPORT.md) | Contenção, O(n), pool/timeouts, cardinalidade de métricas |
| 5 | [SECURITY_REPORT.md](SECURITY_REPORT.md) | Auth, JWT, API-key, multi-tenant, rate-limit, segredos |
| 6 | [IMPROVEMENT_ROADMAP.md](IMPROVEMENT_ROADMAP.md) | P0–P3, risco × alavancagem, critério de pronto |
| 7 | [COMMUNITY_ACCEPTANCE.md](COMMUNITY_ACCEPTANCE.md) | Os 6 pontos de aceitação da comunidade Rust (checklist) |

## Veredicto de uma linha

A **qualidade de engenharia Rust é genuinamente boa** (clippy 0 warnings, 21/25 crates
`#![forbid(unsafe_code)]`, 1 `unsafe` total e em teste, 348 testes, CI robusto). O que
impede a adopção hoje é **1 bug crítico real no consumo de eventos in-process** (corrigido nesta
remessa), **a estabilidade de contrato (sem `#[non_exhaustive]`)**, e o **último quilómetro de
maturidade open-source**. *(Uma segunda suspeita — a macro `controller` fazer panic no arranque —
foi refutada na implementação: ver AUDIT §3.1.)*
**Não adoptaria hoje — por distribuição e maturidade, não por qualidade de código.**

## Nota de honestidade (falsos positivos desarmados)

Três "alarmes" de grep ingénuo **não se confirmam** e são registados para não enganar:

| "Alarme" | Realidade medida |
|---|---|
| "26 `unsafe`" | **1** bloco, num teste. 21/25 crates com `#![forbid(unsafe_code)]`. |
| "11 crates sem testes" | Falso — módulos `#[cfg(test)]` inline. **348 testes** no total. |
| "234 `unwrap` + 158 `expect`" | Em caminho de biblioteca ≈ **54**; panics sobre input não-confiável ≈ **0**. |

> Evidência durável bruta por lane: `scratchpad/findings/lane-{A..F}.md` (fora do repo).

# Perfil de release LLVM

O Seiva afina o perfil **release** do Cargo (§9) para se apoiar no LLVM através do
`rustc`/Cargo — sem magia de build feita à mão. Os botões, no `Cargo.toml` do
workspace:

```toml
[profile.release]
lto = "thin"        # cross-crate optimisation (ThinLTO: most of the win, far
                    # cheaper than fat LTO)
codegen-units = 1   # one unit per crate so LLVM sees the whole crate at once
strip = true        # drop symbols from the shipped artefact
```

O `panic` mantém-se `unwind` (os testes e qualquer fronteira `catch_unwind`
continuam a funcionar), e **deliberadamente não há `target-cpu=native`**: o binário
release por omissão tem de se manter portável entre os hosts onde um PaaS o
escalona. Afina por-host apenas quando controlas o alvo:

```console
$ RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## Medição

O binário do CLI `seiva`, construído de raiz duas vezes num só host (20 núcleos
x86-64, `rustc 1.96.0`), perfil release, `opt-level = 3` por omissão:

| Perfil | Tamanho do binário | Build limpo (wall) |
|---|---:|---:|
| Baseline (`lto = false`, `codegen-units = 16`, sem strip) | 6334 KiB | 16 s |
| Afinado (`lto = "thin"`, `codegen-units = 1`, `strip`) | **1680 KiB** | 21 s |
| Δ | **−73 %** | **+31 %** |

A queda de tamanho é dominada pelo `strip` (tabelas de debug/símbolos), com o
ThinLTO + `codegen-units = 1` a acrescentar eliminação de código morto entre
crates; esses dois também deixam o LLVM optimizar entre fronteiras de crate para o
runtime, ao custo do build limpo documentado de ~31 % mais longo. Estes são números
de um só host — trata-os como indicativos, não como uma garantia; volta a medir no
teu alvo. O RSS e o throughput do *serviço em execução* pertencem ao benchmark de
carga de produção (PostgreSQL + auth + idempotência + réplicas), não a esta tabela
de tempo-de-compilação.

## PGO (opcional)

A Optimização Guiada por Perfil (PGO) **não** está embutida no perfil por omissão:
precisa de uma carga de trabalho representativa e de uma toolchain correspondente, e
um mau perfil pode regredir. Quando tiveres uma carga representativa, aplica-se o
fluxo padrão de duas passagens (`-Cprofile-generate` → correr a carga de trabalho →
`-Cprofile-use`); mantém o perfil gerado fora do repositório e o binário portável a
menos que também fixes o CPU alvo por host.

## O que isto não é

Um binário mais pequeno e optimizado entre crates é uma propriedade de *build*. Não
prova, por si só, que o serviço seguro é rápido sob carga — a auth, a idempotência,
a base de dados e a observabilidade custam todas — por isso o benchmark de produção
mantém a auth/idempotência **ligadas** e reporta p50/p95/p99, throughput, erros,
CPU e memória em separado.

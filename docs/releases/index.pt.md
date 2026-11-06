# Notas de lançamento

O que mudou, e quando. O registo de engenharia corrente vive em
[progress.md](../progress.md); esta página regista os resumos por lançamento.

## 0.1.0 — primeiro lançamento público (2026-11-05)

O primeiro corte publicado da linha `0.1`. A framework está ligada ponta-a-ponta e
verde no CI, e os crates publicáveis estão no
[crates.io](https://crates.io/crates/delonix-seiva) em `0.1.0`, por isso um projecto
gerado compila standalone contra dependências versionadas.

### Destaques

- **Um contrato por módulo.** `#[seiva::facade]` captura as assinaturas de métodos de
  um módulo num `FacadeDescriptor`; o controller HTTP (`#[seiva::controller]`
  → axum), o documento OpenAPI 3.1 e o `.proto` gRPC são todos *gerados* a partir
  dele — sem reflexão, tudo conhecido em tempo de compilação.
- **Clean Architecture imposta pelo grafo de crates.** Cinco crates por módulo
  (`domain`, `application`, `facade`, `infrastructure`, `interface`); a regra de
  dependência é imposta pelo compilador, não por convenção. A camada de application
  codifica apenas contra a fronteira de trait estável em SemVer `seiva-ports`.
- **Um servidor pronto para produção e cluster.** `seiva-server::serve()` compõe o
  router, `/openapi.json`, `/health` (`Starting → Ready → Draining`), encerramento
  gracioso (ctrl-c + SIGTERM), um host de workers e config 12-factor.
- **Baterias.** Identidade Argon2id (`seiva-auth`), config 12-factor tipada
  (`seiva-config`), HMAC Standard Webhooks (`seiva-webhooks`), despacho CQRS
  (`seiva-cqrs`), CloudEvents + brokers (`seiva-messaging`), idempotência durável,
  rate limiting distribuído (Redis), e uma consola de admin servida a partir dos
  contratos do módulo.
- **DORM como a camada de dados por omissão e opcional** por trás de um gateway,
  mantida fora do workspace público para que os crates da framework compilem sem
  credenciais privadas.
- **Scaffold.** `seiva project new` / `seiva module new` emitem módulos com
  transporte ligado que compilam e respondem `POST → 201` prontos a usar.

### Crates publicados (crates.io, `0.1.0`)

Treze crates de framework: `delonix-seiva` (o entrypoint), `seiva-ports`,
`seiva-macros`, `seiva-config`, `seiva-auth`, `seiva-webhooks`, `seiva-cqrs`,
`seiva-openapi`, `seiva-http`, `seiva-server`, `seiva-messaging`,
`seiva-identity` e `seiva-testing`. Adiciona a framework a um projecto com:

```toml
delonix-seiva = "0.1"
```

Os crates acoplados ao DORM (`seiva-data-dorm`, `seiva-outbox-relay`,
`seiva-rag-worker`, `seiva-mcp`) e os `seiva-cli` / `seiva-grpc` acoplados à demo
são `publish = false` por intenção; vê [publicação](../publishing.md) para a
justificação. O `seiva-cli` instala-se com
`cargo install --git https://github.com/ngolacloud/seiva seiva-cli`.

### Desempenho

Numa i9-13900H de 20 cores sobre loopback (medido a 2026-11-05, ressalvas em
[benchmarks](../benchmarks.md)), o servidor Rust serviu aproximadamente **3.4× o
débito de uma framework Python interpretada** e **16× o de uma stack Python WSGI**
em `GET /health`, e **~4.8× / ~15×** respectivamente num `POST` JSON com validação,
com um p99 abaixo de **0.8 ms** em ambos os endpoints. Volta a medir no teu próprio
hardware antes de citar um rácio.

### Ligações

- Crate: <https://crates.io/crates/delonix-seiva>
- Referência da API (rustdoc): <https://ngolacloud.github.io/seiva/>
- Fonte: <https://github.com/ngolacloud/seiva>

## Relacionado

- [Registo de engenharia](../progress.md) · [Estado da publicação](../publishing.md) ·
  [O processo de lançamento](../internals/release-process.md)

---

*Parte da [documentação do Seiva](../index.md), organizada pelos quatro modos
Diátaxis (tutoriais · guias temáticos · referência · guias práticos).*

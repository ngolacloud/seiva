# Desempenho e prontidão cloud-native

A vantagem estrutural do Seiva é que é **nativo e compilado**: sem interpretador, sem
GC, um único binário. Esta página é onde isso se materializa.

## Sem GC, sem runtime

Um serviço Seiva é um binário Rust sobre um runtime assíncrono (tokio). Não há um
colector de lixo a pausar sob carga nem sobrecarga de interpretador por pedido, pelo
que o débito é alto e — mais importante para uma plataforma crítica — a **latência de
cauda é previsível** (sem stop-the-world do GC no p99). Esta é a aposta "nativo, não
interpretado": o ganho não é apenas o débito médio, é a forma da distribuição de
latência.

## Um binário distroless, não-root

O `Delonixfile` gerado constrói uma imagem **distroless** de várias fases: apenas o
binário e as suas bibliotecas de runtime, sem shell, sem gestor de pacotes, a correr
como uid 65532. Implementar é "copiar um binário", não "entregar um runtime
Python/Node e a sua árvore de dependências". Imagem mais pequena, superfície de
ataque menor, arranque a frio mais rápido. Ver
[tutorial parte 6](../getting-started/shop/part-7-monolith-to-microservices.md).

## Prontidão e drenagem graciosa

`/health/ready` e `/health/live` expõem um ciclo de vida
`Starting → Ready → Draining`; o `SIGTERM` drena os pedidos em curso antes de sair.
Uma actualização contínua não larga um pedido nem serve um antes de o serviço estar
pronto.

## Eficiente por construção

- Os corpos validados são delimitados (um limite de tamanho de corpo por omissão)
  antes de um handler correr.
- A camada de dados usa joins SQL, não buscas por linha (sem N+1 — ver
  [consultas](data-layer/queries.md)), e paginação por keyset para listas.
- O middleware de cache de consultas do CQRS faz cache de leituras onde a invalidação
  é conhecida ([caching](caching.md)).

## Benchmarking

A forma honesta de sustentar a alegação de desempenho é um benchmark publicado e
reproduzível contra serviços equivalentes na mesma carga e hardware (débito e
p50/p95/p99). As razões arquitecturais acima são o *porquê* de os números deverem
favorecer o Seiva, mas os números em si devem ser medidos e publicados, não afirmados.

### Medido

Esse arnês existe agora em [`benchmarks/`](https://github.com/ngolacloud/seiva/tree/main/benchmarks)
(`cd benchmarks && ./run.sh`): um serviço Seiva mínimo contra serviços equivalentes em
frameworks interpretados na mesma carga trivial e **sem base de dados**, de modo a
isolar a sobrecarga do framework. Na máquina de referência (i9-13900H de 20 núcleos,
loopback, `oha` a 50 ligações, 20 s, 5 s de aquecimento) o Seiva serviu um múltiplo do
débito deles, com p99 abaixo de **0,8 ms** tanto no `GET /health` como num `POST`
validado. A tabela completa, versões e ressalvas — incluindo os alvos de comparação
exactos — estão em [Benchmarks](../benchmarks.md) e na
[paisagem competitiva](../competitive-landscape.md). Os valores dependem do ambiente —
volta a correr o arnês para medir o teu próprio hardware.

## Relacionado

- [Configuração](configuration.md) · [Observabilidade](observability.md) · [Paisagem competitiva](../competitive-landscape.md)

---

*Parte da [documentação do Seiva](../index.md).*

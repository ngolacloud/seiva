# Benchmarks: Seiva vs stacks Python interpretados

Esta página sustenta a afirmação "nativo, não interpretado" com números
**medidos**, não com asserções. Compara um serviço Seiva mínimo com serviços
Python interpretados equivalentes — um ASGI, um WSGI — na mesma carga trivial, sem
base de dados, para que a medição isole a *sobrecarga da framework*.

O arnês que produz estes números vive em
[`benchmarks/`](https://github.com/ngolacloud/seiva/tree/main/benchmarks) —
`cd benchmarks && ./run.sh`. Tudo abaixo é reproduzível.

> **Lê as ressalvas.** Estes valores dependem do ambiente e foram medidos numa só
> máquina de desenvolvimento partilhada, sobre loopback. Mostram a ordem de
> grandeza certa e o fosso estrutural; **não** são um título universal "o Seiva é N×
> mais rápido". Volta a medir no teu próprio hardware antes de citares um rácio.

## O que foi medido

Três serviços, cada um a expor os **mesmos dois endpoints com lógica idêntica** e
**sem base de dados** (um contador atómico em processo faz de repositório — o
padrão em memória do scaffold do Seiva, espelhado em Python):

- `GET /health` — routing + serializar um corpo JSON minúsculo.
- `POST /v1/notes` — fazer parse de `{"title"}`, rejeitar um título vazio com
  `422`, atribuir um id monótono, ecoar `{"id","title"}` com `201`.

| Serviço | Runtime | Servidor |
|---|---|---|
| Seiva | Rust, compilado `--release` | tokio multi-thread (todos os cores), embutido |
| ASGI interpretado | CPython 3.12, interpretado | uvicorn (uvloop+httptools), 20 workers |
| WSGI interpretado | CPython 3.12, interpretado | gunicorn, 20 workers sync |

Dois controlos de segurança do Seiva foram desligados para a corrida para que a
comparação seja equitativa com as apps Python sem auth: o **limitador de taxa** por
IP (o seu padrão de 300 req/60s responderia a um gerador de carga quase todo com
`429`) e a **guarda de auth** no `POST /v1/notes`. Ambos são features de segurança,
não velocidade de processamento de pedidos; vê `benchmarks/README.md` para a
justificação completa.

## Resultados

Medido a 2026-11-05. **Os números aplicam-se apenas a esta configuração:**

- Host: 20 CPUs lógicos, 13th Gen Intel(R) Core(TM) i9-13900H, Linux.
- Carga: `oha` 1.16.0, **50 ligações, 20 s medidos, 5 s de warmup descartados**,
  HTTP/1.1 keep-alive, sobre **loopback** (o gerador partilha CPU com o servidor).
- Versões: uvicorn 0.32, gunicorn 23, CPython 3.12 (as frameworks ASGI e WSGI nos
  seus lançamentos então correntes).
- Workers Python: 20 (= contagem de CPU) para ambos os serviços Python.

### GET /health (liveness trivial; sem corpo)

| Framework | Pedidos/seg | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| **Seiva (Rust)** | **271,132** | **0.148** | **0.415** | **0.708** |
| ASGI interpretado (uvicorn) | 79,128 | 0.488 | 1.542 | 2.738 |
| WSGI interpretado (gunicorn) | 17,086 | 2.485 | 5.622 | 7.929 |

### POST /v1/notes (validar título não-vazio, atribuir id, ecoar JSON)

| Framework | Pedidos/seg | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| **Seiva (Rust)** | **249,475** | **0.163** | **0.443** | **0.779** |
| ASGI interpretado (uvicorn) | 51,763 | 0.772 | 2.230 | 3.749 |
| WSGI interpretado (gunicorn) | 16,217 | 2.647 | 5.788 | 8.107 |

Todas as corridas terminaram com essencialmente zero respostas não-2xx (algumas
dezenas de pedidos em vários milhões foram abortados no prazo de medição).

### Lendo os números

Nesta máquina, nesta configuração:

- No trivial `GET /health`, o Seiva serviu cerca de **3.4×** o débito da stack ASGI
  e cerca de **16×** o da stack WSGI.
- No JSON `POST /v1/notes`, o Seiva serviu cerca de **4.8×** a stack ASGI e cerca de
  **15×** a stack WSGI.
- O fosso de latência de cauda é maior do que o fosso de débito: o p99 do Seiva
  manteve-se abaixo de **0.8 ms** em ambos os endpoints, enquanto o p99 da stack
  ASGI foi **2.7–3.7 ms** e o da stack WSGI **7.9–8.1 ms**.

Esta é a forma esperada de "código nativo compilado vs. um runtime interpretado com
um GIL": a sobrecarga da framework por pedido — não a lógica de negócio, que aqui é
nula — é o que difere.

## Ressalvas honestas

- **Loopback, host partilhado.** O gerador de carga corre na mesma máquina e compete
  por CPU. Um cliente dedicado sobre uma NIC real moveria os números absolutos.
  Trata os valores como direccionais.
- **Micro-benchmark.** Isto isola a sobrecarga da framework sobre um handler quase
  vazio. Serviços reais passam a maior parte do tempo na base de dados e em chamadas
  a jusante; nesses serviços o fosso *relativo* da framework encolhe porque a
  framework é uma fatia menor do total. O ponto aqui é o piso: quão pouco o próprio
  Seiva acrescenta (p99 sub-milissegundo).
- **A contagem de workers e as versões importam.** O débito Python escala com os
  processos worker (20 aqui); os rácios mudariam com uma contagem diferente ou
  lançamentos mais recentes. Reporta sempre a configuração com o número.
- **Não é um rácio universal.** "~3–5× vs a stack ASGI, ~15× vs a stack WSGI" é
  *esta máquina, esta carga*. Volta a correr `benchmarks/run.sh` para medir a tua.

## Reproduz

```bash
cd benchmarks
./run.sh                                   # default: 50 conns, 20s, 5s warmup
CONNECTIONS=100 DURATION=30 WARMUP=10 ./run.sh
```

Os resultados aterram em `benchmarks/results/<timestamp>/` (tabela, JSON `oha` cru
por endpoint, e o log de cada servidor). Vê
[`benchmarks/README.md`](https://github.com/ngolacloud/seiva/tree/main/benchmarks)
para a metodologia e as regras de equidade.

---

*Parte da [documentação do Seiva](index.md). Vê também
[Desempenho](topics/performance.md) e o
[Panorama competitivo](competitive-landscape.md).*

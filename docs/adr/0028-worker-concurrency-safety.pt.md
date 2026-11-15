# ADR 0028 — Segurança de concorrência dos workers: reclamação competitiva com `SKIP LOCKED` e uma transacção atómica reclamar-efeito-marcar, para que escalar um worker a N réplicas não duplique efeitos

- Estado: Proposto
- Data: 2026-11-15
- Decisores: NgolaCloud
- Segue: ADR 0011 (idempotência durável), ADR 0021 (barramento de eventos in-process e coreografia), ADR 0015 (rate limit distribuído)

## Contexto

O caminho HTTP do Seiva escala horizontalmente sem cerimónia: é stateless e o rate
limit é distribuível (ADR 0015). Os **workers não são** seguros para escalar da
mesma maneira, e nada no scaffold avisa disso.

- O leitor de outbox drena um lote com
  `SELECT ... WHERE topic = $1 AND id > $2 ORDER BY id LIMIT $3` — **sem
  `FOR UPDATE SKIP LOCKED`, sem lease** (`crates/seiva-outbox-relay/src/lib.rs`).
  O `advance` só garante que o cursor avança (`... AND position < $3`). A
  monotonia impede o cursor de *recuar*; **não** impede duas réplicas de lerem e
  processarem o **mesmo** lote. Corre-se `(consumer, topic)` em dois pods e ambos
  reencaminham os mesmos eventos.
- O caminho de publicação no broker (`forward`) **não tem dedup do lado da
  publicação**: N réplicas publicam cada evento N vezes. É at-least-once (os
  consumidores deduplicam via inbox), mas a amplificação é desperdício e carga reais.
- O despachante in-process faz reclamar → processar → finalizar **sem transacção**
  (`crates/seiva-events/src/lib.rs`): um crash entre `processar` e `finalizar` pode
  perder a marca, pelo que o efeito correu mas o evento parece não processado. A
  atomicidade reclamar+efeito+marcar («Fase 2») está **documentada mas não
  implementada** em árvore.
- O próprio comentário do crate admite que a eleição de líder está fora de âmbito:
  *«O laço envolvente (intervalo, encerramento gracioso, métricas, eleição de
  líder) é o runtime de worker do chamador»* — portanto o framework não entrega
  primitivo nenhum para isso.

As forças:

- O contorno seguro de hoje (um Deployment HTTP com N réplicas + um Deployment de
  worker **separado** fixado em `replicas: 1`) é uma armadilha operacional: o óbvio
  `replicas: 3` num Deployment que inclui o worker duplica silenciosamente efeitos
  e carga de BD. O framework tem de tornar **escalar o worker a opção segura por
  omissão**, não uma armadilha.
- A garantia tem de ser **at-least-once com dedup**, nunca at-most-once: perder um
  evento é pior do que processá-lo duas vezes, quando todo o subscritor com efeito
  já deduplica pelo inbox durável (ADR 0011). Logo a correcção é sobre **não
  duplicar trabalho** e **não perder a marca**, não sobre entrega exactly-once (que
  é ficção sobre um broker).
- Tem de ficar dentro dos ports que a aplicação já codifica (`UnitOfWork`,
  `IdempotencyStore`, `EventPublisher`) e do gateway DORM — sem nova dependência de
  infraestrutura (sem ZooKeeper/etcd para eleição de líder).
- O Postgres já oferece o primitivo certo (`FOR UPDATE SKIP LOCKED`), pelo que
  consumidores competitivos podem partilhar uma fila **sem** o estrangulamento de um
  único líder — preferível à eleição de líder, que limita o débito a um só worker.

## Decisão

Tornar o outbox durável uma **fila reclamada competitivamente** e tornar o
processamento in-process **atómico**, para que os workers sejam seguros a N réplicas
por construção.

**1. Reclamação competitiva com `SKIP LOCKED` (substitui o cursor monotónico no caso
multi-leitor).** O `DormOutboxReader` reclama um lote dentro de uma transacção:

```
SELECT ... FROM <outbox>
 WHERE topic = $1 AND status = 'pending'
 ORDER BY id
 LIMIT $3
 FOR UPDATE SKIP LOCKED
```

depois marca as linhas reclamadas `status = 'processing'` com um `owner` e uma
expiração de lease `claimed_at`, e faz commit. Duas réplicas a correr o mesmo
`(consumer, topic)` tiram lotes **disjuntos** — o trabalho é particionado, não
duplicado, sem líder e sem estrangulamento de leitor único. Uma linha cujo lease
expire (o owner caiu antes de completar) volta a `pending` e é re-reclamada —
at-least-once, nunca presa. O cursor forward-only mantém-se apenas como optimização
do lado da leitura onde um único leitor está garantido.

**2. Reclamar-efeito-marcar atómico (a «Fase 2» que o barramento de eventos
documenta).** O despachante in-process corre o subscritor dentro de **uma**
`UnitOfWork::transaction`: reclama a entrada do inbox (dedup, ADR 0011) → corre o
efeito do subscritor → marca processado — tudo com commit conjunto. Um crash em
qualquer ponto faz rollback de tudo, pelo que o evento fica reclamável e é
reprocessado; o efeito e a sua marca nunca podem divergir. Este é o gémeo
in-process da garantia transaccional do outbox, para que um subscritor se comporte
de forma idêntica num monólito e depois de `module split`.

**3. Idempotência do lado da publicação no caminho do broker.** O `forward` carimba
cada evento com uma chave de idempotência estável (id do envelope) para que um
broker/consumidor possa colapsar a amplificação de N réplicas; combinado com (1), a
amplificação é largamente removida na origem porque só a réplica que reclama
reencaminha uma dada linha.

**4. Honestidade do scaffold até isto chegar.** Até (1)–(3) aterrarem, o projecto
gerado leva uma nota explícita e um Deployment de worker fixado em `replicas: 1`,
para que o inseguro `replicas: N` seja uma escolha deliberada, não um acidente.

## Consequências

- Escalar o worker torna-se seguro e útil: consumidores competitivos partilham o
  outbox via `SKIP LOCKED`, o débito cresce com as réplicas, e o lote em curso de um
  worker caído é re-reclamado depois do seu lease — sem líder, sem ponto único.
- A separação efeito/marca que podia perder uma marca fica fechada:
  reclamar-efeito-marcar é uma só transacção, pelo que o sistema é at-least-once com
  dedup durável de ponta a ponta (outbox *e* in-process), coerente com o ADR 0011.
- A amplificação do broker cai de N× para ~1× na origem, com chaves de idempotência
  como rede de segurança do lado do consumidor.
- A garantia é explicitamente at-least-once, não exactly-once: os subscritores com
  efeito têm de se manter idempotentes pelo inbox — o framework impõe o primitivo de
  dedup, o subscritor tem de o usar (inalterado face a ADR 0011/0021).
- O esquema do outbox ganha as colunas `status`, `owner`, `claimed_at`; é uma
  migração DORM, aditiva, aplicada por `seiva db migrate`. O caminho de leitura de um
  deployment de leitor único fica inalterado.
- Não precisa de coordenador externo (sem ZooKeeper/etcd): o primitivo de
  coordenação é o row locking do Postgres, que o gateway DORM já fala.

## Alternativas consideradas

- **Eleição de líder (um worker activo, os outros em standby).** Rejeitado como
  omissão: limita o débito a um único worker e precisa de um lease/coordenador que o
  framework deliberadamente não entrega. O `SKIP LOCKED` dá concorrência segura *e*
  débito horizontal sem coordenador. A eleição de líder mantém-se como escolha de
  deployment válida para um consumidor estritamente ordenado, sobreposta pelo
  operador.
- **Manter o cursor monotónico, fixar o worker em `replicas: 1` para sempre.**
  Rejeitado: faz do worker um estrangulamento de escala e disponibilidade e deixa a
  armadilha no lugar. Aceitável só como a nota interina do scaffold (item 4 da
  decisão).
- **Atribuição estática de partições estilo Kafka.** Rejeitado para o caminho
  durável em árvore: pressupõe um log particionado e um protocolo de rebalanceamento
  que o Seiva não tem. O `SKIP LOCKED` sobre o outbox SQL é a coisa mais simples que
  está correcta aqui; um backend Kafka pode ainda ser um adaptador de mensageria
  atrás do port `Publisher`.
- **At-most-once (marcar antes do efeito).** Rejeitado de imediato: troca trabalho
  duplicado (barato, deduplicado) por efeitos perdidos (perda silenciosa de dados),
  a direcção errada para um framework de alta criticidade.

# A camada de dados (DORM)

> Esta é a **camada de dados** do Seiva — onde vivem os modelos, as consultas e a persistência.

A camada de dados por omissão do Seiva é o **DORM** (o ORM do Delonix), um
**produto independente** ligado ao Seiva apenas através do gateway opcional
`seiva-data-dorm`, por detrás da feature `dorm`. Esta separação é deliberada
(ADR 0001, ADR 0004):

- O **core do framework nunca depende do DORM**. Um módulo cujos repositórios usem
  todos uma fonte de dados externa não puxa ORM nenhum.
- A camada de aplicação programa contra **portas só-trait** (`seiva-ports`:
  `UnitOfWork`, `EventPublisher`, `Clock`, `IdGenerator` e o trait de repositório
  do próprio módulo), nunca contra uma base de dados concreta. É isso que mantém um
  caso de uso testável em unidade sem base de dados (ver [Testes](../testing.md)).

```
application ──(RecordRepository port)──▶ infrastructure
                                            ├─ InMemoryRecordRepository   (default, DORM-free)
                                            └─ DormRecordRepository        (feature = "dorm")
```

## Porquê o DORM (e porquê Rust)

O Prisma — o ORM de referência em TypeScript — passou anos a entregar um motor de
consultas nativo em **Rust** e depois, no Prisma 7 (Nov 2025), substituiu-o por
TypeScript/WASM, porque a **fronteira FFI Rust↔JavaScript** era o estrangulamento.
O DORM não tem essa fronteira: é **nativo em Rust de ponta a ponta** — sem GC, sem
FFI, segurança de tipos em tempo de compilação até ao binário e um deploy de
binário único. A lição que o Prisma aprendeu à força é o ponto de partida do DORM.

## Nesta página / nesta secção

- [Modelos e esquema](models.md) — o DSL do esquema, os blocos `namespace`, `@version`
- [Fazer consultas](queries.md) — consultas tipadas, paginação por keyset, joins (sem N+1)
- [Migrações](migrations.md) — evoluir o esquema com segurança
- [Transacções e a unidade de trabalho](transactions.md) — escritas atómicas, a outbox
- [Múltiplas bases de dados e backends](multiple-databases.md) — encaminhamento, escolha de backend
- [Válvula de escape de SQL cru](raw-sql.md) — consultas cruas tipadas e parametrizadas

## Estado

O DORM é um produto independente em funcionamento (repositório próprio, 130+
testes). A migração Seiva ↔ DORM e o gateway protegido por feature estão em curso;
os adaptadores em memória por omissão deixam-te construir e correr um serviço
completo hoje sem base de dados. Acompanha no [registo de engenharia](../../progress.md) e
na [ADR 0004](../../adr/0004-dorm-data-layer-gateway.md).

---

*Parte da [documentação do Seiva](../../index.md).*

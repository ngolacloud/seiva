# SQL cru

Quando o [construtor de consultas](queries.md) tipado não exprime o que precisas,
o DORM fornece uma válvula de SQL cru **parametrizada**. É uma válvula de escape
deliberada e auditável — não uma porta das traseiras para SQL construído por
strings.

## Quando recorrer a ela

- Uma consulta que o `QuerySet` tipado não consegue representar (um CTE complexo,
  uma função específica da base de dados, um plano afinado à mão).
- Uma instrução pontual de manutenção ou de relatório.

Prefere a camada tipada para tudo o resto: é verificada em tempo de compilação e
portável entre os [backends suportados](backends.md). O SQL cru é a excepção, e
lê-se como tal numa revisão.

## Sempre parametrizado

Os valores são vinculados como parâmetros — `$1`, `$2`, … no PostgreSQL; `?` no
MySQL (o dialecto decide) — e **nunca** intercalados na string de SQL. Essa é a
regra que mantém a válvula segura contra injecção:

```rust
use dorm_core::prelude::*;

// read: values travel as bound parameters, not string interpolation
let rows = db::sql(
    "SELECT id, title FROM records WHERE title = $1",
    &[Value::from(title)],
).await?;
```

As leituras passam por `db::sql` (devolvendo linhas de `Value`); as escritas por
`db::sql_execute` (devolvendo a contagem afectada). A CLI expõe a mesma válvula
como `dorm db execute "<SQL>" -p <value>… [--write]`, onde cada valor `-p` é
vinculado como `$n`, nunca intercalado, e o resultado é renderizado como o
conjunto fechado de tipos que o ORM conhece (projecta qualquer outra coisa como
`::text`).

## Só-leitura por omissão

A válvula crua é só-leitura salvo se aderires: uma escrita precisa do caminho de
escrita explícito (`db::sql_execute`, ou `dorm db execute --write`) e ainda passa
pela [protecção de só-leitura](transactions.md#read-only-mode). Uma flag esquecida
não pode mutar. Na CLI, uma instrução `--write` que seja um `DROP`/`TRUNCATE` nu
ou um `DELETE` sem `WHERE` confirma primeiro (ou precisa de `--yes`); as escritas
dirigidas não perguntam.

## Continua a respeitar a fronteira

O SQL cru corre através do **mesmo âmbito de sessão e de transacção** que as
consultas tipadas, por isso participa na [transacção](transactions.md) activa (e
portanto no `UnitOfWork` quando chamado de um caso de uso Seiva) e em qualquer
encaminhamento primário/réplica. É uma válvula de escape do *construtor de
consultas*, não da disciplina de dados do framework — a transacção, a protecção de
só-leitura e a sessão continuam todas a aplicar-se.

> O DORM limita isto deliberadamente a SQL cru **local**. Uma superfície de "SQL
> nomeado" remota que outros serviços pudessem chamar foi considerada e rejeitada
> (ADR 0058 do DORM): o acesso entre serviços é contratos e eventos, não um
> endpoint SQL partilhado.

## Relacionado

- [Fazer consultas](queries.md) · [Transacções](transactions.md) ·
  [Backends](backends.md)
- Visão de framework: [Válvula de escape de SQL cru](../topics/data-layer/raw-sql.md)

---

*Parte da [documentação do Seiva](../index.md).*

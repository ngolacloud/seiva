# Válvula de escape de SQL cru

Quando a camada de consultas tipadas não exprime o que precisas, o DORM fornece uma
válvula de SQL cru **parametrizada**. É deliberada e auditável (ADR 0005), nunca
uma porta das traseiras para SQL construído em strings.

## Quando recorrer a ela

- Uma consulta que o `QuerySet` tipado não consegue representar (um CTE complexo,
  uma função específica da base de dados, um plano afinado à mão).
- Uma instrução pontual de manutenção ou de relatório.

Prefere a camada tipada para tudo o resto: é verificada em tempo de compilação e
portável entre dialectos pelos [backends suportados](multiple-databases.md).

## Sempre parametrizada

Os valores são vinculados como parâmetros (`$1`, `$2`, … no PostgreSQL; `?` no
MySQL — o dialecto decide), **nunca** inseridos na string de SQL. Esta é a regra
que mantém a válvula segura contra injecção:

```rust
// values travel as bound parameters, not string interpolation
db::fetch("SELECT id, title FROM records WHERE title = $1", &[Value::from(title)]).await?;
```

Uma postura só-leitura é a opção por omissão; uma escrita exige uma adesão
explícita, de modo que uma flag esquecida não possa mutar.

## Continua a respeitar a fronteira

O SQL cru corre através do mesmo âmbito de sessão/transacção que as consultas
tipadas, por isso participa na `UnitOfWork` activa ([transacções](transactions.md))
e no encaminhamento primário/réplica — é uma válvula de escape do *construtor de
consultas*, não da disciplina de dados do framework.

## Relacionado

- [Fazer consultas](queries.md) · [Transacções](transactions.md) · [Múltiplas bases de dados](multiple-databases.md)

---

*Parte da [documentação do Seiva](../../index.md).*

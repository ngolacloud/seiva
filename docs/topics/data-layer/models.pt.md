# Modelos e esquema

A forma dos dados de um projecto Seiva vive num ficheiro, `schema/<project>.schema`,
no DSL de esquema do DORM. Cada módulo em-processo possui um bloco `namespace`; os
modelos lá dentro são as tabelas desse módulo.

## O ficheiro de esquema

`seiva project new` cria o esquema com uma datasource; `seiva module new <m>`
acrescenta um bloco `namespace <m>` com um modelo `Record` inicial:

```
datasource db {
  provider = "postgresql"
}

namespace notes {
  model Record {
    id      Uuid   @id
    title   String
    version Int    @version
  }
}
```

- **`namespace <module>`** — a fronteira do módulo. O DORM suporta namespaces
  nativamente e proíbe relações entre eles, de modo que os dados de um módulo não
  se podem entrelaçar com os de outro: só identificadores atravessam a fronteira (a
  mesma regra que a camada de fachada impõe do lado da API).
- **`model`** — uma tabela. Os campos são `name Type [@attr …]`. Os tipos incluem
  `Uuid`, `String`, `Int`, `BigInt`, `Boolean`, `DateTime`, `Decimal`, `Json`.
- **`@id`** — a chave primária. **`@version`** marca a coluna de concorrência
  optimista (a convenção do framework; ver a nota abaixo). Outros atributos:
  `@unique`, `@index`, `@default(…)`, `@relation(…)`, `@db.VarChar(n)`.

## Do esquema às tabelas

`seiva db dev` reescreve `data/dorm.schema` a partir deste ficheiro, gera o código
dos modelos e redige uma migração a partir dele (ver [migrações](migrations.md)).
Portanto o esquema é a única fonte de verdade: edita-o (ou acrescenta um módulo),
corre `seiva db dev` e a migração segue.

Os nomes dos modelos mapeiam para tabelas pela convenção do DORM (snake_case no
plural): o `Record` do módulo `notes` passa a tabela `records`, `id uuid`,
`title text`, `version` uma coluna inteira.

> **Nota sobre `@version`.** O scaffold marca a coluna de concorrência com
> `@version`. O parser de esquema actual do DORM não aceita esse atributo, por isso
> a sincronização da camada de dados descarta-o (a coluna `version` é na mesma
> criada); a *imposição* da concorrência optimista chega quando o DORM reconhecer o
> atributo.

## Brownfield

Já tens uma base de dados? `seiva db pull` introspecta-a em blocos de modelo que
dobras nos namespaces dos teus módulos — ver [introspecção](../../reference/cli.md) e
a [visão geral da camada de dados](index.md).

## Relacionado

- [Migrações](migrations.md) · [Fazer consultas](queries.md) · [A camada de dados](index.md)

---

*Parte da [documentação do Seiva](../../index.md).*

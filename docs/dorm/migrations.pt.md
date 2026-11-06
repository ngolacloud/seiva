# Migrações

O DORM gera o esquema a partir dos metadados dos teus modelos — o mesmo
`ModelMeta` que as consultas lêem — e aplica-o através de um migrador pequeno e
versionado. Chegas sempre a **ver o SQL antes de ele correr**.

Esta página cobre ambas as visões: o fluxo de trabalho `seiva db *` que usas no
dia a dia, e o migrador DORM por baixo dele.

## O fluxo de trabalho do Seiva

Um projecto Seiva é compatível com o DORM: `seiva db init` prepara a camada de
dados sob `data/`, e o par `seiva db dev` / `seiva db migrate` conduz as vias de
autoria e de implantação do DORM.

```console
$ seiva db init                 # scaffold data/ (dorm.schema, migrations/, the lanes)
$ seiva module new notes        # adds a `namespace notes { model Record … }`
$ seiva db dev add_notes        # sync schema → generate → author + apply a migration
$ seiva db migrate              # deploy: apply the recorded chain (idempotent)
```

`seiva db dev <name>` faz três coisas, por ordem:

1. **sync** — reescreve `data/dorm.schema` a partir do
   `schema/<project>.schema` do teu projecto, para que os modelos de cada módulo
   fiquem actuais;
2. **generate** — regenera o código do modelo a partir desse esquema;
3. **migrate dev** — compara os modelos com o histórico registado em
   `data/migrations/`, escreve a nova migração e aplica-a.

`seiva db migrate` é a via de produção: aplica as migrações já autorizadas em
`data/migrations/`, por ordem, registando cada uma no livro-razão. Não autoriza
nada e não precisa de sincronizar o esquema, por isso é seguro corrê-la em cada
implantação. Ambos os comandos lêem o DSN de `DELONIX_ORM_DSN` (PostgreSQL, ou
qualquer motor com protocolo de fios PG; vê [backends](backends.md)). Estes
comandos delegam na CLI `dorm` — vê
[Usar o DORM a partir do Seiva](using-from-seiva.md).

> O guia de migrações ao nível do framework do Seiva está
> [aqui](../topics/data-layer/migrations.md); esta página é o detalhe DORM por
> baixo dele.

## Gerar DDL

Por baixo, o `dorm-migrations` transforma modelos em texto de `CREATE TABLE` /
`CREATE INDEX`. Cada instrução é emitida com `IF NOT EXISTS`, que é o que permite
ao migrador *adoptar* uma base de dados que já carrega o esquema:

```rust
use dorm_core::model::Model;
use dorm_migrations::ddl;

let models = [<Author as Model>::meta(), <Post as Model>::meta()];
println!("{}", ddl::schema_for(&models)?);          // every table and index
println!("{}", ddl::create_table(<Post as Model>::meta())?);  // one table alone
```

## A migração inicial e o valor `Migration`

`initial` empacota todo o esquema gerado como **versão 1**. Uma `Migration` é um
valor simples e inspeccionável:

```rust
use dorm_migrations as mig;

let m = mig::initial(&models)?;
assert_eq!(m.version, 1);

// pub struct Migration { pub version: i64, pub name: String, pub statements: Vec<String> }
```

> **Um livro-razão, um conjunto de modelos.** `initial` cunha sempre a versão 1.
> Não componhas dois conjuntos de modelos independentes no mesmo histórico de
> migração — dá a cada um a sua própria base de dados/livro-razão, ou funde-os num
> só conjunto de modelos.

## Aplicar migrações

`migrate` aplica cada versão ainda não registada, por ordem, sob um lock
consultivo do PostgreSQL, cada uma dentro da transacção que a regista:

```rust
use dorm_migrations as mig;

// Returns the versions applied this call — empty when everything was current.
let applied = mig::migrate(&[mig::initial(&models)?]).await?;
```

É idempotente de duas maneiras: uma versão já no livro-razão é saltada, e porque
as instruções são `IF NOT EXISTS`, uma primeira execução contra uma base de dados
que já tem o esquema regista a versão **sem alterar nada**. Chamar `migrate` no
arranque de cada processo é, portanto, seguro.

## O livro-razão

As versões aplicadas são registadas numa tabela chamada
**`orm_schema_migrations`**, que o migrador cria sozinho na primeira execução:

| coluna | significado |
|---|---|
| `version` | a versão da migração (chave primária) |
| `name` | o seu nome (`"initial"`, …) |
| `applied_unix` | segundos de relógio de parede quando foi aplicada |

Num projecto Seiva as migrações autorizadas são ficheiros JSON sob
`data/migrations/NNNN_<name>.json` — o SQL, um `down` reversível, e um instantâneo
do esquema. Faz commit desses; o livro-razão vive na base de dados.

## Evoluir o esquema ao longo do tempo

O migrador aplica uma **lista** de migrações por versão. Para mudar o esquema mais
tarde, acrescenta uma nova `Migration` com a versão seguinte — **nunca edites uma
já aplicada**:

```rust
let v2 = mig::Migration {
    version: 2,
    name: "add-posts-views".into(),
    statements: vec![
        "ALTER TABLE posts ADD COLUMN IF NOT EXISTS views BIGINT NOT NULL DEFAULT 0".into(),
    ],
};
mig::migrate(&[mig::initial(&models)?, v2]).await?;
```

Editar o SQL de um passo que uma base de dados já registou não faz nada a essa
base de dados — ficarias com duas bases de dados na mesma versão com esquemas
diferentes, exactamente a falha que uma tabela de versões existe para prevenir. Um
passo errado corrige-se com um passo **novo**.

## `push` vs a cadeia de migração

Existem dois caminhos de autoria, para duas situações:

- **`dorm db push`** (a via de sincronização do `seiva`) sincroniza a base de
  dados com os modelos *aditivamente* — nunca remove nem retipa uma coluna, o que
  **é** a sua protecção contra perda de dados. É para iteração de
  desenvolvimento, não para histórico de produção.
- **A cadeia de migração** (`migrate dev` para autorizar, `migrate deploy` para
  aplicar) é o histórico revisto e versionado que entregas.

O autodetector que autoriza uma migração **recusa um rename ambíguo** — um `DROP`
de uma coluna mais um `ADD` de outra do mesmo tipo, que silenciosamente perderia
dados — até declarares o rename (`@map`) ou o separares em duas migrações. Tipos
diferentes continuam permitidos.

## Estado, rollback e recuperação

- `dorm migrate status` (`--json`) lista o livro-razão das versões aplicadas com
  marcas temporais UTC legíveis.
- `dorm migrate diff` mostra as mudanças aditivas que o `push` aplicaria, sem
  tocar na base de dados.
- `dorm migrate down <version>` inverte uma migração quando é reversível.
- `dorm migrate resolve <version>` marca uma versão como aplicada no livro-razão
  **sem correr o seu SQL** — recuperação depois de uma correcção manual. Recusa
  salvo se nomeares a versão exacta e recusa uma versão já registada.

Estas afloram através da CLI `dorm` em `data/`, e estão a ser levantadas para
subcomandos `seiva db` à medida que o fluxo de trabalho amadurece.

## Relacionado

- [Esquema e modelos](schema-and-models.md) · [Transacções](transactions.md)
- [Backends](backends.md) · [A CLI e o Studio](cli-and-studio.md)

---

*Parte da [documentação do Seiva](../index.md).*

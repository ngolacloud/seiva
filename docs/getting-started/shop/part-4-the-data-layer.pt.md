# Parte 4: A camada de dados

Até agora cada módulo mantém os seus dados em memória. Nesta parte damos à loja
tabelas reais com a camada de dados DORM — o mesmo loop `seiva db` do primeiro
tutorial, agora através de vários módulos. Para a história completa da camada de
dados, ver a [secção DORM](../../dorm/index.md).

## Preparar e migrar

```console
$ seiva db init                 # PostgreSQL by default; or --backend mysql | sqlite
$ export DELONIX_ORM_DSN="host=localhost user=me dbname=shop sslmode=disable"
$ seiva db dev initial
```

`db init` escreve `data/dorm.schema` a partir do `schema/shop.schema` do projecto,
onde cada módulo contribui com o seu próprio `namespace`. No fim desta parte o
schema contém uma tabela por módulo — produtos, stock, encomendas (com os seus
itens de linha), pagamentos, notificações — e o ledger de migrações.

```schema
namespace catalog {
  model Product {
    id          Uuid @id @default(uuid())
    name        String
    price_cents Int
  }
}

namespace inventory {
  model StockItem {
    product_id Uuid @id
    available  Int  @default(0)
  }
}

namespace orders {
  model Order {
    id       Uuid   @id @default(uuid())
    customer String
    status   String @default("placed")
  }
  // order lines reference a product by id — a value owned by `catalog`,
  // referenced across the module boundary by id, not by a foreign-key join.
}
```

O `namespace` de cada módulo é a sua própria fatia da base de dados. Um módulo lê e
escreve apenas as suas próprias tabelas; refere-se aos dados de outro módulo **por
id** (`product_id`), nunca metendo-se nas tabelas do outro módulo. Essa é a mesma
fronteira que o código impõe — levada até à camada de dados.

## Uma transacção, um outbox

Como cada caso de uso corre numa única `UnitOfWork::transaction` e publica o seu
evento dentro dela (Parte 3), o gateway DORM aterra esse evento num **outbox**
transaccional na mesma transacção que a escrita. O worker de relay encaminha-o
depois. Por isso `orders.placed` nunca pode ser emitido para uma encomenda que não
fez commit — o evento e os dados são atómicos.

## Trocar o adaptador

Os módulos ainda usam os seus repositórios em memória no composition root; trocar
cada um pelo seu adaptador suportado por DORM (atrás da feature `dorm`) é a ligação
de produção. Os casos de uso não mudam — já codificam contra a sua **porta** de
repositório. Ver [mudar para DORM](../../howto/switch-to-dorm.md).

## A seguir

[Parte 5: Autorização entre módulos](part-5-authorization.md).

---

*Parte da [documentação do Seiva](../../index.md).*

# Como mudar um módulo para DORM

Um módulo gerado por scaffold usa o repositório em memória. Para o suportar com
uma base de dados real:

1. **Configura a camada de dados** (uma vez por projecto), escolhendo o backend:

   ```console
   $ seiva db init                 # PostgreSQL; or --backend mysql | sqlite
   ```

2. **Migra o modelo do módulo** a partir do esquema do projecto:

   ```console
   $ export DELONIX_ORM_DSN="host=localhost user=me dbname=app sslmode=disable"
   $ seiva db dev initial
   ```

   Isto cria a tabela do módulo a partir do seu `namespace` em
   `schema/<project>.schema` (ver [Migrações](../topics/data-layer/migrations.md)).

3. **Liga o adaptador suportado por DORM** na raiz de composição: por trás da
   feature `dorm`, troca o `InMemoryRecordRepository` do módulo pelo repositório
   suportado por DORM, construído a partir do gateway `seiva-data-dorm`, e
   injecta-o no controlador do módulo (o único lugar que nomeia um adaptador
   concreto). Ver [a camada de dados](../topics/data-layer/index.md).

As camadas de aplicação e de fachada não mudam — codificam contra a porta.

## Relacionado

- [A camada de dados](../topics/data-layer/index.md) · [Múltiplas bases de dados](../topics/data-layer/multiple-databases.md)

---

*Parte da [documentação do Seiva](../index.md).*

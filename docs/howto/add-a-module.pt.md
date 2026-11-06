# Como adicionar um módulo

```console
$ seiva module new billing
```

É esta a receita toda. Ela:

1. gera `modules/billing/` como os cinco crates de camada (facade, domain,
   application, infrastructure, interface);
2. regista-os no `Cargo.toml` do workspace, no `seiva.toml` e no esquema do
   projecto (um bloco `namespace billing` com um modelo inicial);
3. **liga automaticamente** o módulo à raiz de composição — adiciona as
   dependências de interface/infraestrutura ao `apps/<name>-api/Cargo.toml` e
   funde o seu router no `main.rs` (nas âncoras geridas).

Assim a rota do módulo aparece de imediato — confirma com `seiva openapi`.

## Depois

- Edita `modules/billing/facade/src/lib.rs` para dar forma ao contrato (métodos,
  DTOs, `#[requires(...)]`). Ver [Fachadas](../topics/transport/facades.md).
- Edita o bloco `namespace billing` em `schema/<project>.schema` para o modelo, e
  corre `seiva db dev` para o migrar ([Migrações](../topics/data-layer/migrations.md)).

## Relacionado

- [Tutorial parte 1](../getting-started/tutorial/part-1-your-first-module.md)

---

*Parte da [documentação do Seiva](../index.md).*

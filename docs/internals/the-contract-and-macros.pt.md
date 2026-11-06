# O contrato e as macros

As macros do Seiva seguem uma regra (ADR 0001): uma macro mantém o teu código tal
como escrito e apenas **adiciona** itens gerados, e tudo o que gera tem um
equivalente documentado escrito à mão. Corre `seiva expand` para ver a saída de
qualquer entrada.

## `#[seiva::facade(version = "v1")]`

Aplicada a um trait. Reemite o trait sem alterações (menos os atributos auxiliares
`#[requires]`/`#[policy]`/`#[cached]`, que consome) e adiciona:

- um tipo `<Trait>Facade` de tamanho zero que implementa `Facade` com um
  `const DESCRIPTOR` (nome, versão, métodos — cada um com o seu `requires` e
  assinatura);
- um `inventory::submit!` que regista esse descritor para descoberta em tempo de
  link.

O descritor é o [descritor de contrato](../reference/contract-descriptor.md);
nada é reflexivo.

## `#[seiva::controller]`

Aplicada a um `impl` de fachada. Reemite o impl sem alterações e adiciona um
construtor `<trait>_router(service) -> axum::Router`: uma rota por método, o
verbo/caminho a partir da convenção de nome do método, o corpo lido com
`Valid<T>`, a permissão por rota lida do descritor, e o `Ok`/`Err` mapeado para
JSON / RFC 9457. Ver [controladores e encaminhamento](../topics/transport/controllers-and-routing.md).

## `#[derive(WireSchema)]`

Aplicada a uma struct DTO. Implementa `WireSchema::schema()` (um esquema de objecto
composto a partir dos campos) e regista o tipo pelo nome no inventário de
esquemas, para que o gerador OpenAPI resolva a sua forma real. Ver
[wire schema](../reference/wire-schema.md).

## Porquê macros, não reflexão

Os artefactos gerados (OpenAPI, proto, rotas, verificações de permissões) são
todos derivados em tempo de compilação a partir do descritor e das implementações
de `WireSchema`. Não há reflexão em runtime nem registo central a manter
sincronizado — o inventário em tempo de link descobre cada fachada e DTO
compilados no binário. É isto que faz do contrato a única fonte da verdade.

## Relacionado

- [Descritor de contrato](../reference/contract-descriptor.md) · [Wire schema](../reference/wire-schema.md)
- [Fachadas](../topics/transport/facades.md) · [Arquitectura](../architecture.md)

---

*Parte da [documentação do Seiva](../index.md).*

# ADR 0025 — WireSchema reflecte o serde, ou não compila

- Estado: Aceite
- Data: 2026-11-10
- Decisores: NgolaCloud
- Segue: ADR 0007

## Contexto

O `#[derive(WireSchema)]` (ADR 0007) deixa um DTO descrever a sua própria forma
JSON em tempo de compilação, sem reflexão, para que o `/openapi.json`, a consola
`/admin` gerada e o proto gRPC documentem o fio real. O derive lia os campos do
struct e emitia os nomes **Rust** e um `required` baseado em `Option`.

Mas os bytes no fio são produzidos pelo `serde`, e o derive **ignorava todos os
atributos serde**: `#[serde(rename_all = "camelCase")]`, `#[serde(rename = "…")]`,
`skip`, `skip_serializing_if`, `default` e `flatten` movem todos o JSON, e nenhum
alterava o schema publicado. Um DTO que juntava `serde` e `WireSchema` publicava,
portanto, um schema que **discordava em silêncio do JSON que o `axum::Json`
serializa** — o OpenAPI/admin/proto mentia, sem aviso em compilação. O engagement
assinalou isto como o único Crítico restante (SC-1): latente só porque nenhum
exemplo do repositório combinava os dois, mas é o primeiro passo de um utilizador
real.

Um atributo que reformula o valor (`#[serde(with = "…")]`, `serialize_with`,
`deserialize_with`) é ainda pior: muda o *tipo* no fio (p. ex. bytes como string
base64) para algo que o schema não consegue inferir do tipo Rust.

## Decisão

**O derive reflecte os atributos serde que movem o fio, e falha a compilação nos
que não consegue reflectir fielmente.** É reflectir-ou-rejeitar — nunca uma
mentira silenciosa.

Reflectidos (o schema passa a coincidir com o JSON serializado, provado por testes
que comparam os nomes das propriedades do schema com `serde_json::to_value(instância)`):

- `#[serde(rename_all = "…")]` (contentor) — as oito regras de caso do serde,
  aplicadas com as convenções de campo/variante do próprio serde;
- `#[serde(rename = "…")]` (campo e variante de enum) — o nome explícito no fio;
- `#[serde(skip)]` / `skip_serializing` — o campo sai inteiramente do schema;
- `#[serde(skip_serializing_if = "…")]`, `#[serde(default)]` (campo ou contentor),
  `skip_deserializing` — o campo **não** é `required`;
- `#[serde(flatten)]` — as `properties`/`required` do objecto aninhado fundem-se no
  pai, tal como o serde os achata no fio.

Rejeitados em compilação (fariam o schema mentir, logo melhor um erro accionável):
`#[serde(with/serialize_with/deserialize_with)]` num campo e `#[serde(transparent)]`
num contentor. Uma regra `rename_all` desconhecida é igualmente um erro de
compilação em vez de um nome não verificado.

As chaves serde não modeladas que **não** movem a forma do objecto JSON
(`deny_unknown_fields`, `bound`, `crate`, etiquetagem de enum, …) são ignoradas.

## Consequências

- O OpenAPI, a consola de administração e o proto gRPC passam a descrever o fio
  real para um DTO que usa atributos serde — o Crítico SC-1 fica fechado.
- Um DTO que recorra a um atributo serde não suportado que reformula o fio deixa de
  compilar, apontando o autor ao atributo em vez de publicar um schema errado.
- O `schema::Field` ganha um `flatten: bool`; o `object_schema` funde as
  propriedades e as entradas required de um campo achatado no pai.
- O `oneOf` ao nível do valor para enums com dados e as formas base64/`with` ficam
  para trabalho futuro; hoje são rejeitados, não mal-descritos.

# Wire schema

O `WireSchema` deixa um DTO descrever a sua própria forma JSON para que o
documento OpenAPI carregue os **campos reais** do DTO, não um placeholder opaco —
sem reflexão (ADR 0007).

## O trait (`delonix_seiva::schema`)

```rust
pub trait WireSchema { fn schema() -> serde_json::Value; }
```

Implementado no framework para os tipos bem-conhecidos — `String`, `str`, `bool`,
os inteiros (como `{"type":"integer","format":"<ty>"}`), `f32`/`f64`, `()`,
`Vec<T>`, `Option<T>`, e `&T` (que reencaminha) — para que os tipos de campo
componham.

## `#[derive(WireSchema)]`

Numa struct com campos nomeados, ela:

1. implementa `WireSchema::schema()` como um esquema `object` cujas propriedades
   são os campos, o esquema de cada campo composto a partir do seu tipo
   (`<FieldTy as WireSchema>::schema()`); um campo `Option<_>` fica de fora de
   `required`;
2. regista o tipo pelo nome num inventário em tempo de link
   (`SchemaRegistration { name, schema }`).

Rejeita structs de tuplo/unitárias, enums e genéricos (sem mapeamento
nome→forma).

## Resolução

Um gerador que detém uma string de nome-de-tipo resolve o esquema real via
`delonix_seiva::schema::registered_schema(name)`. O `schema_for_rust_type` do
`seiva-openapi` verifica primeiro os escalares, depois o inventário, e só então
recorre ao placeholder honesto `{"type":"string","x-seiva-rust-type":"<ty>"}`
para um tipo nomeado que não derivou `WireSchema`.

## Porque é aditivo

O descritor da fachada continua a carregar apenas `ty: &str`; o `WireSchema`
regista as formas em separado. Por isso adicioná-lo não mudou nenhum descritor e
deixou o gerador de gRPC e as verificações de contrato intocados — um DTO adere
derivando-o, e um que não o faz degrada graciosamente para o placeholder.

## Relacionado

- [Documentos de API](../topics/api-documents.md) · [Validação](../topics/validation.md)
- [O descritor de contrato](contract-descriptor.md) · [ADR 0007](../adr/0007-wire-schema-contract-fidelity.md)

---

*Parte da [documentação do Seiva](../index.md).*

# Validação e DTOs

> A **validação de entrada** do Seiva — entrada validada na fronteira, com erros
> honestos e legíveis por máquina.

Os tipos de fio de um método de fachada são os seus próprios **DTOs** (nunca
entidades de domínio). Um DTO de escrita declara o seu formato com `serde`, as suas
regras com `Validate`, e o seu schema com `WireSchema`.

```rust
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateRecord { pub title: String }

impl Validate for CreateRecord {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.title.trim().is_empty() {
            return Err(ValidationError::new("title must not be empty"));
        }
        Ok(())
    }
}
```

## O extractor `Valid<T>`

O controlador gerado lê um corpo de escrita com `seiva_http::Valid<T>`: desserializa
o JSON para `T` e depois chama `T::validate()`. O corpo **é** o DTO — não há objecto
de envolvimento. Em caso de falha:

- um corpo JSON malformado ou uma incompatibilidade de tipo → `400`;
- um `ValidationError` de `validate()` → **`422 Unprocessable Entity`** como um
  `Problem` RFC 9457 a carregar a tua mensagem.

A validação é um método sobre o tipo, corrida na fronteira, e a falha é uma resposta
de problema tipada — não uma reflexão tardia aparafusada por handler.

## Onde vive a validação

Mantém a validação de *entrada* em `Validate` (formato, campos obrigatórios,
formatos). Mantém as *regras de negócio* na camada de domínio (p. ex. um
`Record::new` que rejeita um título vazio) — o domínio impõe invariantes
independentemente do transporte. O DTO valida o pedido; o domínio valida a verdade.

## Fidelidade de schema

Como o DTO deriva [`WireSchema`](../reference/wire-schema.md), o documento OpenAPI
mostra os seus campos reais e quais são obrigatórios — para que um cliente veja
exactamente o que o `Valid<T>` vai aceitar, sem deriva entre os docs e o servidor
(ADR 0007).

## Relacionado

- [Controladores e encaminhamento](transport/controllers-and-routing.md) · [Pedidos e respostas](transport/request-and-response.md)
- [Documentos da API](api-documents.md) · [ADR 0007 — WireSchema](../adr/0007-wire-schema-contract-fidelity.md)

---

*Parte da [documentação do Seiva](../index.md).*

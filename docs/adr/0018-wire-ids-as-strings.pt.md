# ADR 0018 — Os identificadores viajam no fio como strings JSON

- Status: Aceite
- Data: 2026-11-07
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0007

## Contexto

Os identificadores do Seiva são valores de 128 bits (`seiva_ports::Id` é um
`[u8; 16]`, renderizado como `u128`/UUID/ULID pelos adaptadores). Quando um id
desses é exposto num DTO de fio como um `u128` simples, o `serde` serializa-o como
um **número** JSON.

Um número JSON não é um portador seguro para um inteiro de 128 bits. Na prática os
números JSON são doubles IEEE-754: o JavaScript — o cliente de API mais comum —
representa todos os números como double, por isso qualquer inteiro acima de **2^53**
(`Number.MAX_SAFE_INTEGER`, ~9×10¹⁵) perde precisão silenciosamente no
`JSON.parse`. Um id real do Seiva como `1794005853720908638` (19 dígitos) está bem
acima dessa linha, portanto o browser lê um número *diferente*. Os identificadores
são o pior sítio para isto: um id corrompido aponta para a linha errada, e uma
ida-e-volta (`GET /v1/catalogs/{id}` depois de ler o id de uma lista) devolve `404`
ou, pior, o registo errado. A montra dxjs do tutorial `shop` bateu exactamente
nisto: navegar para um produto pelo id falhava porque o id já tinha sido truncado
na altura do parse no browser.

É um problema de **fidelidade de contrato** no espírito da ADR 0007: o OpenAPI
publicado dizia `{"type":"integer"}` para os ids, mas nenhum cliente JS comum pode
honrar esse tipo para os valores que o servidor realmente emite. As forças:

- O valor no fio tem de ser **exacto** para todos os clientes, incluindo o JavaScript.
- O framework não deve recorrer a reflexão nem a um codec de número à parte (a ADR
  0001 mantém o contrato sem reflexão e todo em tempo de compilação).
- O código interno deve continuar a trabalhar com um tipo inteiro real, não strings.

## Decisão

**Os identificadores viajam no fio como strings JSON.** O framework fornece um
newtype `delonix_seiva::WireId` que embrulha um `u128`, serializa como **string**
decimal, deserializa de uma string (e, com tolerância, de um número para valores
pequenos), e cujo `WireSchema` é `{"type":"string","format":"uint128"}`. Os DTOs de
fio usam `WireId` nos campos de identificador; o domínio mantém o seu `u128` nativo.

```rust
use delonix_seiva::WireId;

#[derive(Debug, Clone, Serialize, Deserialize, WireSchema)]
pub struct ProductDto { pub id: WireId, pub name: String, pub price_cents: u64 }
// no fio: {"id":"1794005853720908638","name":"Mug","price_cents":1500}
```

O `WireId` converte nos dois sentidos com `u128` (`From`/`Into`, `get()`,
`Display`, `FromStr`), por isso a aplicação mapeia um id de domínio para o DTO com
`.into()` e de volta com `.get()` — sem `to_string()`/`parse()` à mão por campo. Um
parâmetro de **caminho** pode continuar `u128` (o axum parseia os dígitos completos
do segmento do URL sem perda de precisão); só os corpos e as respostas JSON
precisam de `WireId`.

Como o `WireId` carrega o seu próprio `WireSchema`, o documento OpenAPI e o Swagger
UI descrevem os ids como strings automaticamente — o documento e o fio voltam a
concordar (ADR 0007).

## Alternativas consideradas

- **Manter os ids como `u128` (números JSON).** O status quo. Rejeitado: é
  silenciosamente errado para o JavaScript e qualquer cliente IEEE-754 acima de
  2^53 — o alvo mais comum — e a falha é um bug de registo-errado difícil de
  rastrear, não um erro.
- **Activar o `arbitrary_precision` do `serde_json`.** Deixa o *servidor* fazer a
  ida-e-volta de um número u128, mas não faz nada pelo *cliente*: o browser continua
  a parseá-lo como um double com perda. E muda o tratamento de números globalmente.
  Rejeitado: não resolve o problema real (do lado do cliente).
- **Um helper de string `#[serde(with = "…")]` por campo.** Funciona, mas deixa o
  `WireSchema` a dizer `integer` a menos que cada campo *também* carregue uma
  sobreposição de schema — duas coisas para lembrar por id, fáceis de fazer meio
  certas (uma string no fio documentada como inteiro). Rejeitado a favor de um tipo
  que carrega a serialização E o schema.
- **Usar sempre um tipo string UUID/ULID.** Uma mudança maior ao modelo de id
  (`seiva_ports::Id`) e aos adaptadores. Fora de âmbito aqui; o `WireId` é sobre
  como um id é *carregado*, não o que ele *é*, e compõe-se com uma futura
  renderização UUID.

## Consequências

- **Fica mais fácil:** qualquer cliente — o JavaScript em primeiro — obtém ids
  exactos de graça; o tipo do OpenAPI bate com o fio; os DTOs lêem-se com
  naturalidade (`id: WireId`); o mapeamento é `.into()` / `.get()`.
- **Fica mais difícil / dívida:** os campos de id são agora strings no JSON, por
  isso um cliente que os tratava (erradamente) como números tem de mudar — uma
  quebra única, positiva para a correcção. O exemplo `shop` adopta o `WireId` como
  referência; outros exemplos que exponham ids devem migrar à medida que forem
  tocados (dívida aceite, 2026-11-07).
- **Imposição:** o `WireId` vive no `delonix-seiva` com testes unitários
  (serialização string, ida-e-volta de valor grande, deserialização string-ou-
  número, schema é `string`). A convenção é documental — DTOs novos escolhem
  `WireId` para ids; os revisores recusam um `u128`/`i64` nu num DTO de fio,
  apontando para aqui.

## Relacionado

- [ADR 0001](0001-seiva-foundations.md) · [ADR 0007](0007-wire-schema-contract-fidelity.md)
- A adopção de referência: [`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop)
  e a montra dxjs separada que a consome.

---

*Parte da [documentação do Seiva](../index.md).*

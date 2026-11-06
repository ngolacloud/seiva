# Parte 2: Validação e DTOs

O módulo `polls` tem uma `Question` e uma `Choice`. No Seiva as formas de
pedido/resposta são **DTOs** na fachada, e esta parte mostra como um corpo é
validado e como isso aparece — honestamente — no contrato.

## Os DTOs

Em `modules/polls/facade/src/lib.rs` o lado de escrita de uma pergunta é um DTO:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateQuestion { pub text: String }

impl Validate for CreateQuestion {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.text.trim().is_empty() {
            return Err(ValidationError::new("question text must not be empty"));
        }
        Ok(())
    }
}
```

- `serde` torna-o o corpo JSON; `WireSchema` publica os seus campos reais no
  OpenAPI; `Validate` é a regra de entrada. `AddChoice { question_id, text }` e
  `Vote { choice_id }` seguem o mesmo padrão.

## Como se comporta

O controlador gerado lê o corpo com `seiva_http::Valid<CreateQuestion>`: o corpo
**é** o DTO (sem invólucro `{cmd: …}`). Por isso:

```console
# valid → 201 Created with the QuestionDto
$ curl -s -XPOST localhost:8080/v1/polls/questions -H 'content-type: application/json' \
    -d '{"text":"What is new?"}'
{"id":1,"text":"What is new?"}

# invalid → 422 Problem (RFC 9457) with your message
$ curl -s -XPOST localhost:8080/v1/polls/questions -H 'content-type: application/json' \
    -d '{"text":"  "}'
{"type":"about:blank","title":"Unprocessable Entity","status":422,
 "detail":"question text must not be empty"}
```

A mesma regra vive também no domínio: `Question::new` rejeita texto vazio, por isso
um comando que escapa pela borda é na mesma recusado pela entidade (a Parte 5 testa
ambos). A validação é defesa em profundidade, não uma única barreira.

## Corresponde à documentação

Como `CreateQuestion` deriva `WireSchema`, `/openapi.json` mostra o corpo como
`{type:object, properties:{text:{type:string}}, required:["text"]}` — exactamente o
que `Valid<CreateQuestion>` aceita. Sem desvio entre a documentação e o servidor
(ADR 0007). Adiciona um campo ao DTO e o contrato acompanha.

## Adicionar um campo (exercício opcional)

Dá a uma pergunta uma data de publicação. Adiciona `pub published_at: String` ao
`QuestionDto` (e leva-o na `Question` do domínio), reinicia e verifica
`/openapi.json` — o novo campo aparece no schema de resposta sem nenhum
serializador para actualizar à mão.

## A seguir

[Parte 3: Persistir com a camada de dados](part-3-the-data-layer.md) — move as
perguntas e opções do adaptador em memória para uma base de dados real.

---

*Parte da [documentação do Seiva](../../index.md).*

# Serialização

O Seiva serializa no fio com **serde**. Os DTOs de uma fachada são os tipos de fio:
o DTO de escrita deriva `Deserialize` (+ `Serialize`), o DTO de resposta deriva
`Serialize`, e ambos derivam [`WireSchema`](../reference/wire-schema.md) para que o
documento OpenAPI corresponda ao que é (des)serializado.

## A regra: DTOs, nunca tipos de domínio

Só os DTOs cruzam a fronteira — nunca uma entidade de domínio. A camada de aplicação
mapeia o domínio para o DTO na borda, pelo que a estrutura interna (e tudo o que seja
sensível) nunca escapa para o fio por acidente. O exemplo `accounts` torna isto
concreto: o seu `UserDto` tem apenas `id` e `username`, pelo que um hash de
palavra-passe *não pode* ser serializado para fora — não há onde ele caiba.

## Versionamento

O contrato é versionado na fachada (`v1`, `v2` lado a lado), pelo que uma alteração
de fio é uma nova versão, não uma quebra silenciosa. Dentro de uma versão, as
alterações aditivas ao DTO (novos campos opcionais) mantêm-se compatíveis; o
documento gerado pelo `WireSchema` mostra aos consumidores exactamente o que esperar.

## JSON hoje

Os corpos HTTP são JSON. O documento proto3 (`seiva proto`) é gerado a partir dos
mesmos descritores para um futuro transporte gRPC; os eventos usam o envelope JSON
CloudEvents (ver [mensageria](messaging-and-events.md)).

## Relacionado

- [Validação](validation.md) · [Documentos da API](api-documents.md) · [Wire schema](../reference/wire-schema.md)

---

*Parte da [documentação do Seiva](../index.md).*

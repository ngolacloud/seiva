# Testes

O Seiva é construído para que as **coisas importantes sejam testáveis sem um servidor
ou uma base de dados**, e para que a **própria arquitectura seja testada** — não
apenas o comportamento.

## Testar unitariamente um caso de uso (sem servidor, sem BD)

A camada de aplicação programa contra portas que são só traits, por isso testas um
caso de uso com fakes em memória do `seiva-testing`:

```rust
#[tokio::test]
async fn create_persists_and_returns_the_dto() {
    let service = RecordService { repository: FakeRepo::default(), ids: FixedIds };
    let dto = service.create(CreateRecord { title: "hello".into() }).await.unwrap();
    assert_eq!(dto.title, "hello");
}
```

Sem HTTP, sem axum, sem o runtime do framework a escapar para a camada de aplicação —
esse é todo o ponto da [camadização](../architecture.md).

## Testes de controlador / HTTP

O `seiva-controller-test` prova que um `#[seiva::controller]` gera um router
funcional, conduzido por fachada, de ponta a ponta: rotas guardadas, caminhos
versionados, erros RFC 9457, e que a autenticação se aplica de facto às rotas de
módulo fundidas.

## Funções de fitness arquitectural

A arquitectura é guardada por testes que falham o build se um invariante for
quebrado — corre-os com `seiva lint`, e vê a skill `seiva-pattern-fitness` para o
catálogo de anti-padrões. Exemplos na árvore:

- a crate `interface` gerada **não** deve depender de `infrastructure` (a regra de
  camadização);
- um projecto gerado tem de **compilar autonomamente** (`cargo build` → 0) — um teste
  de fitness compila de facto um projecto gerado + módulo;
- o corpo OpenAPI gerado tem de ser o DTO directamente, não um envelope `cmd`.

Uma função de fitness que *compila ou corre* a coisa vale mais do que uma que apenas
afirma sobre strings — apanha deriva que uma verificação de texto não apanha.

## Testes de fidelidade de contrato

Como os documentos OpenAPI/proto são gerados, são testados contra os descritores de
fachada reais e os DTOs `WireSchema` reais — para que o contrato publicado não possa
divergir silenciosamente do que o servidor aceita (ver
[Documentos da API](api-documents.md) e [ADR 0007](../adr/0007-wire-schema-contract-fidelity.md)).

## Relacionado

- [Arquitectura](../architecture.md) · [Tutorial parte 5](../getting-started/shop/part-6-testing.md)
- [Contribuir](../internals/contributing.md)

---

*Parte da [documentação do Seiva](../index.md).*

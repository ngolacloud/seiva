# Controladores e encaminhamento

Um **controlador** transforma a implementação de uma fachada em rotas HTTP. Anotas
o `impl` com `#[seiva::controller]`; a macro gera um construtor
`<trait>_router(service)` — nunca escreves uma rota ou um URLconf.

```rust
#[seiva::controller]
impl<R, G> Notes for NotesController<R, G>
where R: RecordRepository + Send + Sync + 'static,
      G: IdGenerator + Send + Sync + 'static,
{
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String> { /* … */ }
}
// generates: pub fn notes_router(service) -> axum::Router
```

## A convenção de encaminhamento

O nome do método escolhe o verbo e o caminho; o recurso é o nome da fachada, em
minúsculas e no plural; a versão é a do descriptor:

| método | HTTP | caminho | pedido |
| --- | --- | --- | --- |
| `create` | `POST` | `/{version}/{resource}` | o corpo do DTO (`201`) |
| `get` | `GET` | `/{version}/{resource}/{id}` | id a partir do caminho |
| `list` | `GET` | `/{version}/{resource}` | consulta opcional |
| *outro* | `POST` | `/{version}/{resource}/{method}` | o corpo do DTO |

## O handler gerado

Para cada rota o handler puxa o serviço injectado a partir do estado do axum,
extrai o pedido, invoca o método da fachada e mapeia o resultado:

- os métodos de **escrita** lêem o único parâmetro de corpo com
  `seiva_http::Valid<T>` — um corpo JSON validado (ver [Validação](../validation.md));
  o corpo **é** o DTO, por isso o OpenAPI documenta-o directamente (sem envelope
  `{cmd: …}`).
- **`get`** lê o id com `axum::extract::Path<T>`; **`list`** um
  `axum::extract::Query<T>` opcional.
- o valor `Ok` é serializado para JSON; o `Err` mapeia para um `Problem` RFC 9457.

## Permissão por rota

A permissão exigida de cada rota é o `#[requires("…")]` do método, lido do
descriptor em tempo de execução e imposto por `seiva_http::permission_guard` —
nunca codificado à mão. Um `requires` vazio é uma rota pública.

## Composição

O controlador é genérico sobre as suas portas, por isso a camada `interface` nunca
nomeia um adaptador concreto — a raiz de composição em `apps/<name>-api`
injecta-os e funde o router (`seiva module new` liga isto automaticamente). A fusão
acontece *antes* das camadas do framework, por isso authn/segurança/rate-limit
aplicam-se às rotas do módulo — ver
[a aplicação e a raiz de composição](application-and-composition.md) e
[middleware e camadas](middleware-layers.md).

## Relacionado

- [Fachadas](facades.md) · [Validação](../validation.md) · [Documentos de API](../api-documents.md)

---

*Parte da [documentação do Seiva](../../index.md).*

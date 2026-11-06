# Autorização ao nível do objecto

A autenticação diz *quem* é quem chama (um `Principal` verificado); a autorização diz
*o que ele pode fazer*. O Seiva tem duas camadas, e empurra a difícil para tempo de
compilação.

## Ao nível da função: `#[requires]`

Cada método de fachada declara a permissão que quem chama tem de deter:

```rust
#[requires("notes.create")]
async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, String>;
```

O `seiva_http::permission_guard` lê isso do descritor em tempo de execução e
impõe-no antes de o handler correr: quem chama sem `notes.create` recebe `403`. Isto
é OWASP API5 (autorização quebrada ao nível da função), fechada por construção — a
permissão viaja com o contrato, não uma lista de middleware escrita à mão.

## Ao nível do objecto: `Scoped<T>` (BOLA)

A permissão ao nível da função não chega: quem tem autorização para ler *uma* nota
continua a não dever ler a nota *de outra pessoa*. Isso é OWASP API1 (Broken Object
Level Authorization), a falha de API mais comum. O `Scoped<T>` do Seiva torna-a uma
preocupação **fail-closed em tempo de compilação** (ADR 0005): um valor que cruza a
fronteira é envolvido de modo a que lê-lo *exija* provar que o âmbito de quem chama
(tenant/dono) corresponde. O código que esquece a verificação não compila, em vez de
servir silenciosamente os dados de outro tenant.

```rust
// A Scoped<Note> cannot be unwrapped without a Principal whose scope it matches;
// an out-of-scope access is an error, never a silent leak.
let note: Note = scoped.for_principal(&principal)?;
```

## Isolamento de tenant, de ponta a ponta

A fronteira de tenant atravessa a pilha: a camada de dados proíbe relações entre
`namespace`s de módulos (DORM-B2), e o `Scoped<T>` impõe o dono por linha na fronteira
do serviço. Juntos tornam o acesso entre tenants um erro de tipo, não uma esperança
de revisão de código.

## Relacionado

- [Autenticação](authentication.md) · [Segurança](security.md)
- [ADR 0005 — BOLA](../adr/0005-object-level-authorization-bola.md) · [ADR 0006 — Identidade](../adr/0006-identity-and-authentication.md)

---

*Parte da [documentação do Seiva](../index.md).*

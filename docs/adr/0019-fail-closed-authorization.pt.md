# ADR 0019 — A autorização é fail-closed: cada método do facade declara a sua política

- Status: Aceite
- Data: 2026-11-08
- Decisores: NgolaCloud
- Segue: ADR 0005, ADR 0006

## Contexto

Um método do facade declara a permissão que o chamador precisa com
`#[requires("perm")]` (ADR 0006); o `#[seiva::controller]` monta o
`permission_guard` a partir do `MethodDescriptor.requires` do método, e o guard
trata um requisito **vazio** como «sem autorização», deixando o pedido passar.

Até agora, o requisito vazio era também o *valor por omissão*: um método sem o
atributo `#[requires(..)]` compilava para `requires: ""` e, portanto, para uma
**rota pública**. A direcção da falha era a perigosa — *esquecer* de guardar um
método não quebrava a compilação, expunha silenciosamente a rota a chamadores
anónimos (OWASP API5, Autorização Quebrada ao Nível da Função). Nada no sistema de
tipos ou na compilação distinguia «deliberadamente público» de «o autor
esqueceu-se». Uma revisão de segurança do framework assinalou isto como o único
valor por omissão capaz de comprometer qualquer serviço construído sobre o Seiva
sem o autor dar por isso.

As forças:

- Uma política de acesso em falta tem de ser **impossível de enviar**, não um
  valor por omissão silencioso.
- Um endpoint genuinamente público (um eco de saúde, uma leitura de catálogo
  pública) tem de continuar exprimível — fail-closed não pode significar «tudo
  guardado».
- A decisão tem de ser **auditável**: um revisor deve ver, no próprio método, que
  a exposição pública foi escolhida de propósito.
- Sem alteração ao fio, às rotas nem à forma do `MethodDescriptor` (ADR 0007): um
  `requires` vazio já é a sentinela pública do guard.

## Decisão

**Cada método do facade tem de declarar a sua política de acesso em tempo de
compilação.** Um método sem `#[requires("perm")]` nem o novo atributo `#[public]`
é um **erro de compilação**, com uma mensagem accionável:

```text
facade method `delete` declares no access policy: add `#[requires("permission")]`
to guard it, or `#[public]` to expose it without authorization (an explicit,
auditable choice)
```

`#[public]` é a saída explícita e auditável do guard de permissão. Não recebe
argumentos e mantém `requires` vazio — a mesma sentinela que o guard já lê como
«sem autorização» — pelo que o `MethodDescriptor`, a rota gerada e o bloco
`security` do OpenAPI ficam byte a byte iguais ao que um `requires` vazio escrito à
mão produzia antes. Só o *valor por omissão* mudou: a ausência de política é agora
um erro, não uma rota pública.

```rust
#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, ctx: &Ctx, cmd: CreateNote) -> Result<NoteDto, String>;

    #[public] // deliberadamente não-autenticado — escolha auditada, não um default
    async fn ping(&self) -> Result<(), String>;
}
```

Esta é a metade da autorização numa ronda mais ampla de endurecimento fail-closed;
as restantes alterações (o `Valid<T>` a respeitar o limite de corpo, a rejeição de
campos desconhecidos, os `500` sem fuga, o rate-limit com escopo de inquilino, os
tokens indexados por SHA-256, a detecção de colisões de rota no arranque) são
internas e não precisam de uma decisão de contrato.

## Alternativas consideradas

- **Manter o default público silencioso.** O status quo. Rejeitado: o modo de
  falha (esquecer `#[requires]` → rota anónima) é exactamente o inverso do que um
  controlo de segurança deve ter, e nada o traz à superfície.
- **Negar em runtime (o guard recusa um requisito vazio).** Tornaria uma política
  esquecida num `403` em vez de uma rota pública, mas também quebra todo o endpoint
  genuinamente público enquanto não for re-marcado, e a falha só aparece quando a
  rota é atingida, não na compilação. Rejeitado: a compilação é mais cedo e total.
- **Um lint ao nível do crate (`seiva lint`) que assinala métodos sem anotação.**
  Apanha o mesmo erro, mas só quando o lint corre, e é consultivo. Rejeitado a
  favor de um erro de compilação rígido que nenhum pipeline consegue saltar.
- **Um `#[requires]` com string vazia como marcador público.** É já como o guard lê
  o público, mas `#[requires("")]` parece um erro de escrita e é fácil de pôr por
  acidente — o oposto de auditável. O `#[public]` nomeia a intenção.

## Consequências

- **Mais fácil:** uma política de acesso esquecida já não pode ser enviada —
  quebra a compilação no próprio método, com uma mensagem que nomeia a correcção. A
  exposição pública é visível no local da chamada (`#[public]`), pelo que um revisor
  a audita directamente.
- **Mais difícil / dívida:** um método de facade existente que dependia do default
  público silencioso tem agora de dizer `#[public]` explicitamente. É uma quebra
  única e positiva para a segurança; os exemplos na árvore e o teste do controlador
  foram migrados com este ADR (todos os facades de exemplo já declaravam
  `#[requires]`, pelo que só os métodos `list`/`ping` deliberadamente públicos
  precisaram do marcador).
- **Imposição:** o gate vive no `#[seiva::facade]` (`seiva-macros`); o teste do
  facade em `delonix-seiva` e o `seiva-controller-test` exercitam um método
  guardado e um `#[public]`, pelo que uma regressão que reabra o default silencioso
  quebra a compilação.

## Relacionado

- [ADR 0005](0005-object-level-authorization-bola.md) · [ADR 0006](0006-identity-and-authentication.md) · [ADR 0007](0007-wire-schema-contract-fidelity.md)

---

*Parte da [documentação do Seiva](../index.md).*

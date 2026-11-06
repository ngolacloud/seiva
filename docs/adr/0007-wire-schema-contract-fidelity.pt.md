# ADR 0007 — Fidelidade de contrato wire-schema: os DTOs descrevem a sua própria forma

- Estado: Aceite
- Data: 2026-11-05
- Decisores: NgolaCloud
- Segue: ADR 0001, ADR 0003

## Contexto

Um descriptor de fachada (`#[seiva::facade]`, ADR 0001) leva os **tipos de
parâmetro e de retorno de cada método como strings de nome de tipo Rust**
(`"CreateNote"`, `"Result<NoteDto, String>"`), nunca uma descrição estrutural — a
regra "zero reflexão". O gerador de OpenAPI (`seiva-openapi`) produzia, por isso,
um contrato **infiel** de duas maneiras:

1. **O envelope `cmd`.** O corpo do pedido de um método de escrita era
   documentado como um `object` cujas propriedades eram os parâmetros indexados
   por nome (`{"properties":{"cmd":…},"required":["cmd"]}`). Mas o
   `#[seiva::controller]` gerado desserializa o corpo **como o DTO do parâmetro
   directamente** (`seiva_http::Valid<CreateNote>`) e rejeita mais do que um
   parâmetro de corpo. Um cliente que seguisse o documento enviaria `{"cmd":{…}}`
   e o servidor rejeitá-lo-ia — as formas documentada e aceite divergiam.

2. **O marcador `x-seiva-rust-type`.** Para qualquer tipo não escalar os schemas
   de corpo e de resposta eram `{"type":"string","x-seiva-rust-type":"CreateNote"}`
   — reivindicando uma *string* onde o fio leva um *object* JSON com campos
   nomeados.

Juntos, estes são o bloqueador de "fidelidade de contrato": o OpenAPI gerado não
era um contrato contra o qual um cliente pudesse codificar. As forças: o gerador
não consegue ver dentro de um tipo nomeado sem ou reflexão (rejeitada pela ADR
0001) ou o tipo a descrever-se a si próprio; e tudo o que acrescentarmos não pode
quebrar a forma do descriptor que o gerador gRPC e as verificações
`[[uses]]`/`[[allow]]` já lêem.

## Decisão

**Um DTO contribui com o seu próprio JSON Schema através de um trait
`WireSchema`, derivado em tempo de compilação e resolvido pelo gerador através de
um inventário em tempo de ligação — o mesmo mecanismo que o `#[seiva::facade]` já
usa para descobrir fachadas. O gerador de OpenAPI documenta um corpo de escrita
como o DTO do parâmetro directamente, não um envelope.**

1. **`delonix_seiva::schema::WireSchema` (novo).** `fn schema() ->
   serde_json::Value` devolve um fragmento de JSON Schema. Implementado aqui para
   os escalares bem conhecidos, `Vec<T>`, `Option<T>` e `&T` (para que os tipos
   dos campos componham), espelhando `seiva_openapi::schema_for_rust_type` de modo
   que o schema de um campo seja idêntico quer alcançado por nome quer por
   composição.

2. **`#[derive(WireSchema)]` (novo, em `seiva-macros`, re-exportado como
   `delonix_seiva::WireSchema`).** Para uma struct com campos nomeados implementa
   `WireSchema` (um schema `object` cujas propriedades compõem o
   `<FieldTy as WireSchema>::schema()` de cada campo; um campo `Option<_>` é
   omitido de `required`) **e** emite um `inventory::submit!` que regista o nome
   do tipo → schema. Rejeita structs de tuplo/unidade, enums e genéricos, que não
   têm mapeamento nome→forma para registar.

3. **O gerador resolve por nome, de forma aditiva.** `schema_for_rust_type`
   procura um tipo nomeado via `delonix_seiva::schema::registered_schema` antes de
   recorrer ao marcador. Um tipo que **não** derivou `WireSchema` está ausente do
   inventário e continua a documentar-se honestamente como marcador. **Nenhum
   campo de descriptor mudou** — `Param`/`MethodDescriptor` mantêm `ty: &'static
   str` — por isso o gerador gRPC e as verificações de contrato ficam intocados.

4. **O corpo é o DTO.** Para um `POST` com exactamente um parâmetro de corpo wire
   (não-`&Ctx`), o schema do corpo do pedido é o schema desse parâmetro
   directamente; com nenhum, não há corpo. O antigo object-indexado mantém-se
   apenas como recurso para um descriptor feito à mão que declare mais do que um
   parâmetro de corpo (inalcançável através de `#[controller]`).

Os DTOs de exemplo (`notes`, `accounts`) e o template de módulo gerado derivam
`WireSchema`, por isso cada fachada que o framework entrega e gera documenta a
sua forma real.

## Alternativas consideradas

- **Enfiar um fornecedor de schema no descriptor** (`Param.schema: fn() ->
  Value`, emitido por `#[facade]`). Rejeitada: altera a forma do `const
  DESCRIPTOR` lida pelo gerador gRPC e pelas verificações de contrato, força um
  bound `WireSchema` em *cada* tipo de parâmetro/retorno (incluindo `&Ctx`), e
  complica o equivalente feito à mão — um raio de explosão bem maior do que o
  inventário por nome, para o mesmo resultado.
- **Uma crate de meta-schema/reflexão (p. ex. `schemars`).** Rejeitada pela regra
  de zero reflexão da ADR 0001 e pela postura sem-DORM/dependências-finas;
  `WireSchema` é um derive, resolvido em tempo de compilação.
- **Deixar o marcador, corrigir só o envelope.** Rejeitada: remover o envelope
  deixa um marcador `type:string` nu a reivindicar que o corpo inteiro é uma
  string — mais enganador do que antes. Os dois defeitos estão acoplados.

## Consequências

- O OpenAPI gerado é agora um contrato fiel: `seiva openapi` documenta o corpo de
  criação de `notes` como `{title}` e a resposta como `{id,title}`, com zero
  marcadores `x-seiva-rust-type` para DTOs `WireSchema`.
- Um DTO de fachada tem de derivar `WireSchema` para ser documentado
  estruturalmente; um que não o faça degrada-se graciosamente para o marcador
  nomeado (sem quebra).
- A resolução é em tempo de ligação, como a descoberta de fachadas: uma
  ferramenta autónoma vê só os DTOs compilados nela; o binário de runtime que
  monta os módulos produz o documento completo. Dois DTOs que partilhem um nome
  curto entre crates colidem (o primeiro a corresponder ganha) — uma propriedade
  pré-existente dos descriptors indexados por nome.
- A fidelidade gRPC/proto está **inalterada** (lê `ty` e mantém o seu próprio
  mapeamento); enriquecer a mensagem proto a partir de `WireSchema` é um possível
  seguimento.
- `Vec<u8>` compõe-se como um array de inteiros em vez do caso especial
  `format:"byte"` do mapeador de strings — uma divergência menor e documentada
  para schemas de campo compostos.

## Adenda (2026-11-05) — enums, bytes e respostas de erro

Estendendo o trabalho de fidelidade (§8) sem alterar a decisão acima:

- **Enums de variantes de unidade.** `#[derive(WireSchema)]` aceita agora um enum
  estilo-C e emite `{"type":"string","enum":[…nomes de variantes…]}`
  (`schema::enum_schema`), correspondendo à serialização por omissão do serde de
  uma variante de unidade como o seu nome. Uma variante **que carrega dados** é um
  erro de compilação claro (um `oneOf` fiel é trabalho futuro), e o
  `rename`/`rename_all` do serde não é reflectido — nomeie as variantes do enum
  wire como se serializam. Enums aninhados ficam inline no object que os contém.
- **Bytes.** `Vec<u8>` compõe-se como um array de inteiros — fiel à forma wire por
  omissão do `serde_json` (uma string base64 `format:"byte"` precisaria de um
  atributo serde), agora coberto por um teste.
- **Respostas de erro.** O gerador de OpenAPI documenta as respostas de erro como
  `application/problem+json` referenciando um schema `Problem` partilhado em
  `components.schemas`: uma operação protegida documenta `401`/`403`, e qualquer
  operação com corpo de pedido documenta `422`. `components` está, portanto,
  sempre presente (leva o `Problem`); o esquema de bearer só é acrescentado quando
  um método é protegido.
- **gRPC** mantém-se só geração de proto; um round-trip servido não é
  explicitamente reivindicado, por isso nenhum é afirmado.

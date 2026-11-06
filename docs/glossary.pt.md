# Glossário

**Fachada** — o contrato público versionado de um módulo: um trait anotado com
`#[seiva::facade]`. A única fonte de verdade a partir da qual os transportes, o
documento OpenAPI e as verificações de permissão são derivados.

**FacadeDescriptor** — a forma legível por máquina de uma fachada (nome, versão,
métodos, a permissão `requires` e a assinatura de cada método), gerada pela macro e
lida pelos geradores. Ver [o descritor do contrato](reference/contract-descriptor.md).

**Controlador** — o adaptador de transporte, anotado `#[seiva::controller]`, que
transforma uma impl de fachada num router axum (`<trait>_router`).

**Módulo** — uma unidade da aplicação, feita de cinco crates de camada (facade,
domain, application, infrastructure, interface). É dono dos seus dados e expõe um
contrato.

**Camada / a regra de dependência** — a direcção fixa das dependências entre os
cinco crates, imposta pelo compilador. Ver [Arquitectura](architecture.md).

**Porta** — uma fronteira só-trait (em `seiva-ports`: `UnitOfWork`,
`EventPublisher`, `Clock`, `IdGenerator`, mais o trait de repositório de um módulo)
contra a qual a camada de aplicação programa, para ser testável sem servidor nem
base de dados.

**DORM** — o ORM da Delonix, a camada de dados por omissão do Seiva, um **produto
independente** ligado através da feature opcional `dorm`. O core da framework nunca
depende dele.

**DTO** — um data transfer object: o tipo de wire explícito (nunca uma entidade de
domínio), derivando `serde`, `Validate` e `WireSchema`.

**`Valid<T>`** — o extractor que desserializa e valida um corpo de pedido,
produzindo um `422` RFC 9457 em caso de falha.

**WireSchema** — o trait (e `#[derive(WireSchema)]`) pelo qual um DTO contribui com
o seu JSON Schema real para o documento OpenAPI, sem reflection. Ver
[ADR 0007](adr/0007-wire-schema-contract-fidelity.md).

**Principal** — uma identidade de chamador verificada (subject, inquilino,
permissões), produzida por um `Authenticator` a partir de um token bearer. Ver
[Autenticação](topics/authentication.md).

**Authenticator** — o trait com negação por omissão que transforma um bearer num
`Principal` (`TokenAuthenticator` para dev, `JwtAuthenticator` para OIDC/JWT).

**`Scoped<T>`** — o invólucro fail-closed que torna a autorização ao nível do
objecto uma preocupação de tempo de compilação (prevenção de BOLA). Ver
[Autorização](topics/authorization.md).

**`permission_guard` / `#[requires("…")]`** — autorização ao nível da função: a
permissão que um chamador tem de possuir para uma rota, lida do descritor.

**`router_with_modules`** — o construtor da aplicação que funde os routers dos
módulos **antes** de aplicar as camadas da framework, para que a
autenticação/segurança/rate-limit se apliquem às rotas dos módulos. Ver
[a camada de transporte](topics/transport/index.md).

**Composition root** — `apps/<name>-api`: onde os adaptadores concretos são
escolhidos e os módulos são montados num serviço. Não contém lógica de negócio.

**Função de fitness** — um teste que guarda uma invariante arquitectural e falha o
build se ela for quebrada (corre com `seiva lint`).

**ADR** — um Architecture Decision Record. Ver [o índice de ADRs](internals/adrs.md).

---

*Parte da [documentação do Seiva](index.md).*

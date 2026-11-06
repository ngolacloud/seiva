# Documentos da API: OpenAPI, as UIs dos docs, admin e proto

O Seiva é API-first: em vez de renderizar templates HTML, **gera os próprios
documentos da API** a partir dos contratos de fachada — sem spec mantida à mão, sem
anotações por handler. A `async fn` na fachada *é* a fonte: o seu DTO de pedido, o
seu DTO de resposta e a sua permissão tornam-se o caminho, os schemas e o requisito
de segurança. O resultado são docs interactivos que nunca escreveste — entregues a
partir de um contrato compilado.

## OpenAPI 3.1 (`/openapi.json`)

Todo o serviço em execução serve `GET /openapi.json`, gerado a partir de **todas as
fachadas ligadas ao binário** via o inventário em tempo de ligação (sem lista
central — OWASP API9). É *fiel*: um corpo de escrita é o DTO directamente (sem
envelope `{cmd:…}`), e os campos reais de cada DTO vêm do seu derive `WireSchema` —
não um marcador opaco (ADR 0007). As operações guardadas carregam `x-seiva-requires`
e um esquema de segurança bearer.

Offline, a CLI imprime o documento do projecto:

```console
$ seiva openapi            # builds+runs the project, emits its OpenAPI; --compact for one line
```

Dentro de um projecto isto documenta *os teus* módulos (via `SEIVA_EMIT=openapi` na
raiz de composição); fora de um projecto mostra a fachada de demonstração integrada.

## As UIs interactivas dos docs (`/docs`, `/redoc`, `/scalar`)

Todo o serviço em execução serve três páginas interactivas de referência sobre o seu
próprio `/openapi.json`, montadas por omissão — Swagger UI e ReDoc, mais um renderer
de bónus:

| Rota      | Renderer     | Notas    |
| ---------- | ------------ | -------- |
| `/docs`    | Swagger UI   |          |
| `/redoc`   | ReDoc        |          |
| `/scalar`  | Scalar       | bónus    |

Chega ao Swagger UI em `/docs` para o painel completo *Try it out* — e não escreveste
spec, nem anotações, nem página. Os bundles do visualizador carregam de um CDN; cada
página define a sua própria Content-Security-Policy delimitada para que o estrito
`default-src 'none'` do plano de dados não a deixe em branco, enquanto cada resposta
de API/JSON mantém essa predefinição estrita.

## A consola de administração (`/admin`) — uma extensão opcional

A consola de administração **não** faz parte do framework base (ADR 0008). Um backend
Seiva simples responde `404` para `/admin` e não carrega nenhum dos seus assets. A
consola é entregue como uma crate separada, `seiva-console-admin`, à qual uma raiz de
composição adere explicitamente:

```rust
// in your apps/<name>-api composition root, merge into `modules` BEFORE the
// framework layers wrap them (so /admin gets authn, headers and the rate limit):
let modules = seiva_console_admin::mount(modules);
let app = seiva_http::router_with_modules(health, &security, auth, modules);
```

O `seiva ext add seiva-console-admin` insere essa linha na âncora gerida por ti
(ADR 0010).

Uma vez montado, `GET /admin` é uma consola autónoma (sem CDN, CSP estrita baseada em
hash) que renderiza cada recurso **registado e autorizado** com as suas operações e
um formulário de tentativa por operação de escrita, construído a partir do schema do
DTO, com um campo de token bearer.

Um recurso aparece apenas quando um módulo o **regista** com
`delonix_seiva::register_admin!` *e* quem chama detém a sua permissão `view` — estar
no `/openapi.json` nunca é suficiente (ADR 0009). A consola lê um
`GET /admin/registry.json` filtrado pelo servidor, pelo que quem chama sem
autorização não vê nem o recurso nem a sua existência; os formulários de escrita
ficam ocultos a menos que quem chama detenha a permissão de criação/alteração (e a
guarda do servidor impõe-no de qualquer forma). Um módulo que não regista nada
mantém-se privado da consola.

```rust
// in the module's interface layer (e.g. examples/accounts/interface):
use delonix_seiva::admin::{AdminPermissions, AdminResource};
use delonix_seiva::register_admin;

register_admin!(AdminResource {
    namespace: "accounts", resource: "accounts", label: "Accounts",
    facade: "Accounts", version: "v1",
    list_fields: &["id", "username"], search_fields: &["username"],
    filters: &[], ordering: &["username"], page_size: 25,
    permissions: AdminPermissions {
        view: "accounts.read", create: "accounts.create", change: "", delete: "",
    },
});
```

Conduz as *operações* do contrato; para navegar/editar linhas de dados usa
`seiva db studio`. Ver a história [do admin](authentication.md) para o modelo de auth.

## proto3 (`seiva proto`)

```console
$ seiva proto              # the project's proto3 service document
```

Espelha o `seiva openapi`: dentro de um projecto emite as fachadas do projecto como
serviços proto (`SEIVA_EMIT=proto`); fora, a demonstração integrada. Servir o
transporte gRPC é uma fase posterior; o documento proto é gerado a partir do mesmo
descritor hoje.

## Porque nunca deriva

Os quatro documentos derivam dos *mesmos* descritores de fachada e `WireSchema` que o
servidor impõe em tempo de execução. Não há uma segunda fonte de verdade para manter
sincronizada — os documentos são o contrato, gerados.

## Relacionado

- [Fachadas](transport/facades.md) · [Validação](validation.md)
- [Wire schema](../reference/wire-schema.md) · [ADR 0007](../adr/0007-wire-schema-contract-fidelity.md)

---

*Parte da [documentação do Seiva](../index.md).*

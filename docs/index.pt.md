# Documentação do Seiva

<div class="seiva-hero" markdown>
# Delonix Seiva
Uma framework de backend em Rust para sistemas robustos e de alta criticidade —
**tratamento ergonómico de pedidos** e **baterias incluídas**, entregue como um
único binário compilado, sem GC. Os teus handlers são simples `async fn`; um
módulo é dono dos seus dados e expõe **um contrato legível por máquina**, e a
partir dele o Seiva *deriva* as rotas HTTP, a documentação interactiva da API, as
verificações de permissões e o serviço gRPC — por isso escreves comportamento,
não boilerplate.
</div>

> **Novo por aqui?** Lê o [Seiva num relance](getting-started/seiva-at-a-glance.md),
> e depois o [Guia de instalação rápida](getting-started/quick-install.md). Vens
> de uma framework dinâmica? Começa pelo [teu primeiro dia](first-day.md). Queres
> construir algo ponta-a-ponta? O [tutorial](getting-started/shop/part-1-the-modular-monolith.md)
> constrói uma app de sondagens como monólito e depois separa um módulo num
> microserviço.

## Porquê o Seiva

<div class="grid cards" markdown>

-   :material-flash: **Assíncrono por omissão**

    Os handlers são `async fn` nativos sobre um runtime Tokio multi-thread — um
    modelo mental assíncrono familiar, sem GC e sem surpresas de event-loop.

-   :material-file-document-check: **Documentação que nunca escreves**

    Cada serviço serve `/openapi.json` e **Swagger UI** interactivo (`/docs`),
    **ReDoc** (`/redoc`) e **Scalar** (`/scalar`) prontos a usar, gerados a partir
    do contrato. Sem anotações, sem especificação para manter.

-   :material-shield-lock: **Seguro e tipado ponta-a-ponta**

    Permissões com negação por omissão por método, problemas RFC 9457, e DTOs
    cujos campos reais são publicados no OpenAPI — o compilador mantém o fio
    honesto.

-   :material-arrange-send-backward: **Monólito *ou* microserviços**

    Começa como um binário modular; promove qualquer módulo ao seu próprio serviço
    com `seiva module split` quando precisares — o mesmo código, um botão de
    implantação.

</div>

## Como a documentação está organizada

Esta documentação está organizada em quatro modos — o modelo
[Diátaxis](https://diataxis.fr) — porque mantém o *aprender* separado do
*consultar*. (Vê [o mapa da estrutura](STRUCTURE.md) para perceberes como as
secções se encaixam.)

<div class="grid cards" markdown>

-   :material-school: **[Tutoriais](getting-started/index.md)**

    ---

    Lições orientadas à aprendizagem, guiadas passo a passo, que te levam de um
    checkout vazio a um serviço a correr, autenticado e empacotado.

    [:octicons-arrow-right-24: Começar o tutorial](getting-started/shop/part-1-the-modular-monolith.md)

-   :material-book-open-variant: **[Guias temáticos](topics/index.md)**

    ---

    Discussões orientadas à compreensão de cada camada: a camada de dados, o
    transporte, a validação, a identidade, a segurança, a observabilidade, os
    eventos.

    [:octicons-arrow-right-24: Usar o Seiva](topics/index.md)

-   :material-file-document-multiple: **[Referência](reference/index.md)**

    ---

    Descrições orientadas à informação, exaustivas, da CLI, das definições, do
    descritor de contrato e dos crates.

    [:octicons-arrow-right-24: Consultar](reference/index.md)

-   :material-wrench: **[Guias práticos](howto/index.md)**

    ---

    Receitas orientadas a tarefas para problemas específicos: adicionar um módulo,
    mudar para DORM, implantar distroless.

    [:octicons-arrow-right-24: Receitas](howto/index.md)

</div>

### Destaques

<div class="grid cards" markdown>

-   :material-sitemap: **[Arquitectura](architecture.md)**

    ---

    As camadas, o contrato de fachada, e os transportes gerados a partir dele.

-   :material-speedometer: **[Benchmarks](benchmarks.md)**

    ---

    Débito e latência medidos no caminho de produção, contra stacks
    interpretados.

</div>

## Primeiros passos

- [Seiva num relance](getting-started/seiva-at-a-glance.md)
- [Guia de instalação rápida](getting-started/quick-install.md) · o
  [quickstart](quickstart.md) mais completo
- O tutorial: [1 · O teu primeiro módulo](getting-started/shop/part-1-the-modular-monolith.md)
  → [2 · Validação](getting-started/shop/part-2-modules-own-their-data.md)
  → [3 · Camada de dados](getting-started/shop/part-4-the-data-layer.md)
  → [4 · Autenticação](getting-started/shop/part-5-authorization.md)
  → [5 · Testes](getting-started/shop/part-6-testing.md)
  → [6 · Empacotamento & implantação](getting-started/shop/part-7-monolith-to-microservices.md)
  → [7 · Do monólito ao microserviço](getting-started/shop/part-7-monolith-to-microservices.md)
- [A vir de uma framework dinâmica](first-day.md)

## As camadas (guias temáticos)

Um guia por preocupação que um backend tem de resolver:

| O que precisas | Guia |
| --- | --- |
| Guardar e consultar dados | [A camada de dados (DORM)](topics/data-layer/index.md) |
| Expor HTTP & gRPC | [A camada de transporte](topics/transport/index.md) |
| Validar dados de entrada | [Validação e DTOs](topics/validation.md) |
| Documentar a API | [Documentos da API (OpenAPI / UI de docs / proto)](topics/api-documents.md) |
| Autenticar & autorizar | [Autenticação](topics/authentication.md) · [Autorização](topics/authorization.md) |
| Endurecer por omissão | [Segurança](topics/security.md) |
| Ver o que se passa | [Observabilidade](topics/observability.md) |
| Reagir a eventos & correr trabalho | [Mensagens & eventos](topics/messaging-and-events.md) · [Workers em segundo plano](topics/background-workers.md) |
| Separar leituras de escritas | [Despacho CQRS](topics/cqrs.md) |
| Testar sem servidor nem BD | [Testes](topics/testing.md) |
| Configurar para produção | [Configuração](topics/configuration.md) |
| Correr rápido & cloud-native | [Desempenho & cloud-native](topics/performance.md) |

## Referência

- [A CLI `seiva`](reference/cli.md)
- [Definições e ambiente](reference/settings.md)
- [O descritor de contrato](reference/contract-descriptor.md) ·
  [Esquema de fio](reference/wire-schema.md)
- [Os crates](reference/crates/index.md)

## O projecto Seiva

- [Arquitectura](architecture.md) e os [detalhes internos de desenho](internals/index.md)
- [Registos de Decisões de Arquitectura](internals/adrs.md) (o directório [`adr/`](adr/0001-seiva-foundations.md))
- [FAQ](faq/index.md) · [Glossário](glossary.md)
- [Panorama competitivo](competitive-landscape.md) e o [roadmap](roadmap.md)
- [Notas de lançamento](releases/index.md) e o [registo de engenharia](progress.md)
- [Estado da publicação](publishing.md) · [Contribuir](internals/contributing.md)

---

*Esta documentação segue o modelo [Diátaxis](https://diataxis.fr).
As páginas marcadas como “planeadas” trazem o esboço pretendido; as capacidades da
framework são seguidas com honestidade em [progress.md](progress.md) e nos ADRs.*

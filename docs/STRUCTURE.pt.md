# Estrutura da documentação — o layout Diátaxis

A documentação do Seiva está organizada pelos quatro modos
[Diátaxis](https://diataxis.fr) — **tutoriais · guias temáticos · referência ·
how-to** — o modelo que mantém a documentação de uma framework navegável: material
de aprendizagem, material de compreensão, material de informação e material de
tarefa vivem cada um no seu lugar. O objectivo: qualquer programador de backend
deve orientar-se pela documentação do Seiva por instinto.

Esta página mapeia as secções de topo e as camadas temáticas, e nota onde a forma
do Seiva é distintiva (o Seiva é API-first e compilado, por isso algumas
preocupações — templates HTML, um admin com GUI, i18n — mapeiam para coisas
diferentes ou estão fora de âmbito por agora).

## Secções de topo

| Secção | Caminho | Notas |
| --- | --- | --- |
| Primeiros passos | [`getting-started/`](getting-started/index.md) | num relance, instalação, o tutorial |
| Guias temáticos («Usar o Seiva») | [`topics/`](topics/index.md) | um guia orientado à compreensão por camada |
| Guias how-to | [`howto/`](howto/index.md) | receitas de tarefas |
| Referência da API | [`reference/`](reference/index.md) | CLI, definições, descritor, crates |
| FAQ | [`faq/`](faq/index.md) | |
| Glossário | [`glossary.md`](glossary.md) | |
| Notas de versão | [`releases/`](releases/index.md) | + [registo de engenharia](progress.md) |
| Pormenores internos | [`internals/`](internals/index.md) | arquitectura, ADRs, como contribuir |

## As camadas temáticas (guias temáticos)

| Preocupação | Tópico do Seiva | Estado |
| --- | --- | --- |
| A camada de dados | [A camada de dados (DORM)](topics/data-layer/index.md) | O DORM é um produto separado, ligado via a feature `dorm` |
| A camada de transporte | [A camada de transporte](topics/transport/index.md) | fachadas + controladores, gerados — não vistas escritas à mão |
| Documentos de API | [Documentos de API](topics/api-documents.md) | **distintivo:** o Seiva é API-first → OpenAPI/UI de docs/proto em vez de templates HTML |
| Validação | [Validação e DTOs](topics/validation.md) | `Valid<T>` + `Validate`, erros RFC 9457 |
| O processo de desenvolvimento | [Testes](topics/testing.md), [Configuração](topics/configuration.md), [Empacotamento](topics/performance.md) | |
| Admin | — | **fora de âmbito (por agora):** um admin auto-CRUD é um item do roadmap; ver a análise competitiva |
| Segurança | [Segurança](topics/security.md), [Autorização](topics/authorization.md) | negação por omissão, BOLA por construção |
| Internacionalização | — | fora de âmbito por agora |
| Desempenho e optimização | [Desempenho e cloud-native](topics/performance.md) | binário único, distroless |
| Ferramentas comuns de aplicações web | [Autenticação](topics/authentication.md), [Cache](topics/caching.md), [Observabilidade](topics/observability.md), [Mensageria](topics/messaging-and-events.md), [Paginação](topics/pagination.md), [Email](topics/sending-email.md), [Sessões](topics/sessions.md), [Serialização](topics/serialization.md) | |
| Outras funcionalidades core | [Sinais/eventos](topics/messaging-and-events.md), [CQRS](topics/cqrs.md), [Workers em segundo plano](topics/background-workers.md) | |
| O projecto open-source | [Pormenores internos](internals/index.md), [ADRs](internals/adrs.md), [Como contribuir](internals/contributing.md), [Processo de release](internals/release-process.md) | |

## Como navegar

Procura **getting-started** para aprender a fazer, **topics/** para perceber uma
camada em profundidade, **reference/** para consultar uma flag ou definição exacta,
e **howto/** para uma receita focada. A [página inicial da documentação](index.md)
é o ponto de entrada.

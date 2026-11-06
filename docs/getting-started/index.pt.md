# Primeiros passos

Novo no Seiva? Começa por aqui. Estas páginas são **orientadas à aprendizagem**:
segue-as por ordem e passarás de um checkout vazio a um serviço a correr,
validado, autenticado e empacotado.

## Primeiros passos

1. [O Seiva num relance](seiva-at-a-glance.md) — o que é o Seiva, a grande ideia,
   e o que recebes à partida.
2. [Guia de instalação rápida](quick-install.md) — instala a CLI `seiva` e cria um
   projecto. (A versão mais completa é o [quickstart](../quickstart.md).)
3. [A chegar de uma framework dinâmica](../first-day.md) — mapeia cada hábito que
   trazes ao seu equivalente no Seiva, marcando honestamente o que existe hoje e o
   que está em curso.

## O tutorial

Uma construção guiada de uma aplicação **polls** (perguntas, opções, votação,
resultados) — uma preocupação por parte, orientada à aprendizagem, de um checkout
vazio até um microserviço separado:

1. [O teu primeiro módulo](tutorial/part-1-your-first-module.md) — gera o módulo polls e conhece as cinco camadas
2. [Validação e DTOs](tutorial/part-2-validation-and-dtos.md) — `Valid<T>`, RFC 9457, `WireSchema`
3. [Persistir com a camada de dados](tutorial/part-3-the-data-layer.md) — troca para o DORM
4. [Autenticação e permissões](tutorial/part-4-authentication.md) — `Principal` verificado, `#[requires]`
5. [Testar o teu módulo](tutorial/part-5-testing.md) — fakes, testes de controlador, funções de fitness
6. [Empacotamento e deploy](tutorial/part-6-packaging-and-deploy.md) — distroless, não-root, `/health/ready`
7. [De monólito a microserviço](tutorial/part-7-monolith-to-microservice.md) — `seiva module split`, o mesmo código autónomo
8. [Um frontend no browser com dxjs](tutorial/part-8-a-dxjs-frontend.md) — uma UI reactiva segura em CSP a consumir a API polls

## Para onde a seguir

[O que ler a seguir](what-to-read-next.md) — os guias temáticos, a referência e os
pormenores internos do desenho.

---

*Parte da [documentação do Seiva](../index.md).*

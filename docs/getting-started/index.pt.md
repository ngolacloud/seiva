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

## O tutorial: construir uma loja

Uma construção guiada de uma **loja de comércio electrónico** de cinco módulos
(catalog · inventory · orders · payments · notifications) — uma preocupação por
parte, de um checkout vazio até um backend que corre como monólito *e* como
microserviços, mais uma montra de browser. Os módulos coordenam-se por **eventos**,
não por chamadas.

1. [Um monólito modular](shop/part-1-the-modular-monolith.md) — gerar a loja e os seus cinco módulos
2. [Os módulos possuem os seus dados](shop/part-2-modules-own-their-data.md) — os cinco contratos e as suas permissões
3. [Eventos entre módulos](shop/part-3-events-between-modules.md) — o fluxo da encomenda, ligado por eventos
4. [A camada de dados](shop/part-4-the-data-layer.md) — tabelas reais por módulo com o DORM
5. [Autorização entre módulos](shop/part-5-authorization.md) — quem pode fazer o quê, imposto pelo contrato
6. [CQRS](shop/part-6-cqrs.md) — separar leituras de escritas com o `seiva-cqrs`
7. [Docs de API & Swagger](shop/part-7-api-docs-and-swagger.md) — cada fluxo documentado, sem spec escrita à mão
8. [Testar a loja](shop/part-8-testing.md) — fakes, o fluxo de eventos, funções de fitness
9. [Do monólito aos microserviços](shop/part-9-monolith-to-microservices.md) — separar `payments` + `notifications`, o mesmo código
10. [Uma consola admin](shop/part-10-admin-console.md) — `seiva-console-admin` como extensão, sem frontend escrito
11. [Um frontend dxjs separado, com CORS](shop/part-11-a-separate-dxjs-frontend.md) — uma montra desacoplada na sua própria origem

## Para onde a seguir

[O que ler a seguir](what-to-read-next.md) — os guias temáticos, a referência e os
pormenores internos do desenho.

---

*Parte da [documentação do Seiva](../index.md).*

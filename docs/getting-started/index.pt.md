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
6. [Testar a loja](shop/part-6-testing.md) — fakes, o fluxo de eventos, funções de fitness
7. [Do monólito aos microserviços](shop/part-7-monolith-to-microservices.md) — separar `payments` + `notifications`, o mesmo código
8. [Um frontend de browser com dxjs](shop/part-8-a-dxjs-frontend.md) — uma montra segura sob CSP a consumir a API da loja

## Para onde a seguir

[O que ler a seguir](what-to-read-next.md) — os guias temáticos, a referência e os
pormenores internos do desenho.

---

*Parte da [documentação do Seiva](../index.md).*

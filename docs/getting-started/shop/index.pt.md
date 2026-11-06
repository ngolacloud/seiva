# O tutorial: construir uma loja

Este tutorial constrói uma **pequena loja de comércio electrónico** a partir de
**cinco módulos** e mostra a recompensa da fronteira de módulo do Seiva: o mesmo
código corre como **um monólito modular** e, quando precisas, como
**microserviços independentes** — por composição, não por reescrita. Começas com
um checkout vazio e terminas com um backend que escala de um binário a uma frota
de serviços, mais uma montra de browser.

O código terminado é o
[`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop) no
repositório — cinco módulos de cinco camadas que compilam e estão testados.

## A loja

| Módulo | Possui | Faz |
| --- | --- | --- |
| `catalog` | produtos | adicionar um produto, listar, obter um |
| `inventory` | níveis de stock | receber stock, reservar stock, verificar disponibilidade |
| `orders` | encomendas | fazer uma encomenda (linhas de produto + quantidade), obter, listar |
| `payments` | pagamentos | cobrar uma encomenda, obter um pagamento |
| `notifications` | mensagens enviadas | enviar uma notificação, listar |

Nenhum módulo chama o código de outro módulo. Coordenam-se através de **eventos de
domínio**: fazer uma encomenda emite `orders.placed`; isso desencadeia uma reserva
de stock (`inventory.reserved`), uma cobrança (`payments.captured`) e uma
confirmação (`notifications.sent`). Cada módulo possui os seus dados e o seu
contrato; o fluxo é ligação, não acoplamento.

## As partes

1. [Um monólito modular](part-1-the-modular-monolith.md) — gerar a loja e os seus cinco módulos.
2. [Os módulos possuem os seus dados](part-2-modules-own-their-data.md) — os cinco contratos e as suas permissões.
3. [Eventos entre módulos](part-3-events-between-modules.md) — o fluxo da encomenda, ligado por eventos.
4. [A camada de dados](part-4-the-data-layer.md) — dar à loja tabelas reais com o DORM.
5. [Autorização entre módulos](part-5-authorization.md) — quem pode fazer o quê, imposto pelo contrato.
6. [Testar a loja](part-6-testing.md) — testes unitários, o fluxo de eventos, funções de aptidão.
7. [Do monólito aos microserviços](part-7-monolith-to-microservices.md) — separar `payments` e `notifications` nos seus próprios serviços.
8. [Um frontend de browser com dxjs](part-8-a-dxjs-frontend.md) — uma montra segura sob CSP que consome a API da loja.

---

*Parte da [documentação do Seiva](../../index.md).*

# O tutorial: construir uma loja

Este tutorial constrói uma **pequena loja de comércio electrónico** a partir de
**cinco módulos** e mostra o proveito da fronteira de módulo do Seiva: o mesmo
código corre como **um monólito modular** e, quando for preciso, como
**microserviços independentes** — por composição, não por reescrita. Começas com
um checkout vazio e terminas com um backend que escala de um binário a uma frota
de serviços, mais uma montra no navegador.

O código acabado é
[`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop) no
repositório — cinco módulos de cinco camadas que compilam e estão testados.

## A loja

| Módulo | Possui | Faz |
| --- | --- | --- |
| `catalog` | produtos | adicionar um produto, listar, obter um |
| `inventory` | níveis de stock | receber stock, reservar stock, verificar disponibilidade |
| `orders` | encomendas | colocar uma encomenda (linhas de produto + quantidade), obter, listar |
| `payments` | pagamentos | cobrar uma encomenda, obter um pagamento |
| `notifications` | mensagens enviadas | enviar uma notificação, listar |

Nenhum módulo chama o código de outro módulo. Coordenam-se através de **eventos de
domínio**: colocar uma encomenda emite `orders.placed`; isso conduz uma reserva de
stock (`inventory.reserved`), uma cobrança (`payments.captured`) e uma confirmação
(`notifications.sent`). Cada módulo possui os seus dados e o seu contrato; o fluxo
é ligação, não acoplamento.

## As partes

1. [Um monólito modular](part-1-the-modular-monolith.md) — montar a loja e os seus cinco módulos.
2. [Os módulos possuem os seus dados](part-2-modules-own-their-data.md) — os cinco contratos e as suas permissões.
3. [Eventos entre módulos](part-3-events-between-modules.md) — o fluxo da encomenda, ligado por eventos.
4. [A camada de dados](part-4-the-data-layer.md) — dar à loja tabelas reais com DORM.
5. [Autorização entre módulos](part-5-authorization.md) — quem pode fazer o quê, imposto pelo contrato.
6. [CQRS](part-6-cqrs.md) — separar leituras de escritas com `seiva-cqrs`.
7. [Documentação de API auto-gerada & Swagger](part-7-api-docs-and-swagger.md) — cada fluxo documentado, sem escrever especificação.
8. [Testar a loja](part-8-testing.md) — testes unitários, o fluxo de eventos, funções de aptidão.
9. [Do monólito aos microserviços](part-9-monolith-to-microservices.md) — separar `payments` e `notifications` nos seus próprios serviços.
10. [Uma consola admin](part-10-admin-console.md) — `seiva-console-admin` como extensão, sem escrever frontend.
11. [Um frontend dxjs separado, sobre CORS](part-11-a-separate-dxjs-frontend.md) — uma montra desacoplada na sua própria origem.

Cada parte é complementar: diz **onde vive o código**, os **comandos para
construir e testar**, e corre sob **`seiva start --watch`** com live-reload.

---

*Parte da [documentação do Seiva](../../index.md).*

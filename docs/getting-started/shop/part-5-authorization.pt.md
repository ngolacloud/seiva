# Parte 5: Autorização entre módulos

Cada método na loja declara a permissão que o chamador tem de possuir. Com cinco
módulos, essas permissões tornam-se um pequeno mapa de capacidades — e porque cada
uma é declarada no contrato, o transporte impõe-na da mesma forma em todo o lado,
por negação por omissão.

## As permissões da loja

| Módulo | Permissão | Guarda |
| --- | --- | --- |
| catalog | `catalog.manage` · `catalog.read` | adicionar produtos · navegar |
| inventory | `inventory.manage` · `inventory.read` | receber/reservar stock · verificar disponibilidade |
| orders | `orders.manage` · `orders.read` | colocar encomendas · vê-las |
| payments | `payments.charge` · `payments.read` | cobrar · ver um pagamento |
| notifications | `notifications.send` · `notifications.read` | enviar · listar |

Um token que carregue `catalog.read` pode navegar o catálogo mas não adicionar um
produto; um token de back-office com `catalog.manage` pode fazer ambos. A permissão
`payments.charge` é deliberadamente estreita — a reacção de cobrança no fluxo da
encomenda corre sob uma identidade de serviço que possui exactamente isso, nada
mais.

## Negação por omissão

Sem autenticador ligado, cada rota guardada responde `403` — nenhum chamador é
alguma vez autenticado, por isso nada é permitido. Aponta o servidor para o JWKS do
teu fornecedor OIDC e ele verifica os tokens bearer (RS256/ES256, `iss`/`aud`/`exp`
verificados) e lê as permissões de um claim configurável:

```console
$ export SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
$ export SEIVA_OIDC_ISSUER=https://id.example.com/
$ export SEIVA_OIDC_AUDIENCE=shop
$ seiva start
```

| Pedido | Resultado |
| --- | --- |
| sem bearer | `anonymous` — rotas guardadas `403` |
| bearer inválido | `401` + `WWW-Authenticate` |
| bearer válido | a permissão `<module>.<action>` da rota é verificada |

## Autorização ao nível do objecto

A permissão ao nível da função diz *quem pode chamar*; a autorização ao nível do
objecto diz *que linhas podem tocar* — para uma loja multi-inquilino, as encomendas
de que comerciante um chamador pode ver. O `Scoped<T>` do Seiva torna o acesso
entre inquilinos uma preocupação em tempo de compilação, fechada por omissão. Ver
[Autorização](../../topics/authorization.md) e
[Segurança](../../topics/security.md).

## Seguinte

[Parte 6: CQRS — separar leituras de escritas](part-6-cqrs.md).

---

*Parte da [documentação do Seiva](../../index.md).*

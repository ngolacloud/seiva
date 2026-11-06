# Pedidos e respostas

Como um pedido flui através de um serviço Seiva, e como os resultados e os erros
voltam no wire.

## O ciclo de vida

Um pedido entra nas camadas do framework (a mais externa primeiro), chega a uma
rota de módulo, e o handler gerado corre o caso de uso:

```
request → request-id → security headers → authn → rate limit → CORS →
          body-size limit → route handler → facade method → response
```

As camadas estão cobertas em [middleware e a pilha de camadas](middleware-layers.md);
esta página é sobre o que o handler lê e devolve.

## Ler o pedido

O handler `#[seiva::controller]` extrai exactamente o que o método precisa:

- um corpo de **escrita** é desserializado e validado como o DTO da fachada
  (`seiva_http::Valid<T>`) — o corpo **é** o DTO (ver [Validação](../validation.md));
- um **`get`** lê o id a partir do caminho; um **`list`** lê uma consulta opcional.

Um corpo de tamanho excessivo (acima do limite por omissão de 1 MiB) é rejeitado
antes de o handler correr.

## Respostas

- **Sucesso** — o valor `Ok` da fachada é serializado para JSON com o estado da
  convenção: `201 Created` para `create`, `200 OK` caso contrário.
- **Falha** — o erro da fachada mapeia para um `Problem` RFC 9457
  [`Problem`](../../reference/crates/index.md) (`application/problem+json`): um erro
  legível por máquina com um `type`, `title`, `status` e `detail`. As falhas de
  validação tornam-se `422 Unprocessable Entity`; um `#[requires]` não satisfeito é
  `403` (ou `401` quando foi fornecido um bearer mas inválido).

Cada resposta — incluindo uma que foi curto-circuitada (um `429`, um `401`, um
preflight CORS) — carrega os cabeçalhos seguros e um `request-id` de correlação,
porque essas camadas são as mais externas.

## Tipo de conteúdo

Os pedidos e as respostas são JSON. Os DTOs derivam `serde`, e
[`WireSchema`](../../reference/wire-schema.md) publica as suas formas reais no
documento OpenAPI, para que um cliente programe contra os corpos exactos que o
servidor aceita e devolve.

## Relacionado

- [Controladores e encaminhamento](controllers-and-routing.md) · [Middleware e camadas](middleware-layers.md)
- [Validação](../validation.md) · [Segurança](../security.md)

---

*Parte da [documentação do Seiva](../../index.md).*

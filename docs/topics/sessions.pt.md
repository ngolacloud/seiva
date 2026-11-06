# Sessões

Os serviços Seiva são **baseados em tokens** por omissão, não em sessões — o que é a
predefinição certa para uma API.

## Prefere tokens

Um pedido autentica-se com um token bearer, verificado por pedido num `Principal`
([Autenticação](authentication.md)). Não há um armazém de sessões do lado do servidor
a manter, replicar ou invalidar — o token carrega a identidade e o serviço mantém-se
sem estado, que é o que lhe permite escalar horizontalmente e implementar como um
único binário sem sessões aderentes.

## Quando precisas genuinamente de estado de sessão

Alguns fluxos têm estado (um assistente de vários passos, um carrinho de compras, um
login renderizado no servidor). Modela esse estado como **dados** — uma linha que o
teu módulo detém, com chave no subject do `Principal` — através da camada de dados
normal, não de um objecto de sessão mágico. Assim é migrado, consultado e autorizado
como quaisquer outros dados (incluindo verificações `Scoped<T>` ao nível do objecto),
e sobrevive a um reinício.

Para um *token* de sessão opaco (em vez de um JWT), aplica-se a mesma costura
`Authenticator`: um adaptador ao estilo `TokenAuthenticator` pode resolver um token
opaco para um `Principal` contra o teu armazém.

> Âmbito: o Seiva traz o caminho de tokens (JWT/OIDC verificado) como predefinição;
> um auxiliar de sessão por cookie com baterias incluídas não faz parte do framework
> hoje.

## Relacionado

- [Autenticação](authentication.md) · [Configuração](configuration.md)

---

*Parte da [documentação do Seiva](../index.md).*

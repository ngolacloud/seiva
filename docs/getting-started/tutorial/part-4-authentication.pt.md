# Parte 4: Autenticação e permissões

A fachada polls declara uma permissão em cada método — `polls.manage` para criar
perguntas e opções, `polls.vote` para votar, `polls.read` para listar e ler
resultados. Nesta parte vemos o que esses guardas significam e como admitir
chamadores reais. (Onde outras frameworks aparafusam a autenticação por rota, a
permissão do Seiva é declarada no contrato e imposta pelo transporte.)

## As rotas estão guardadas por omissão

Sem nenhum authenticator ligado, a framework é de **negação por omissão**: uma rota
guardada responde `403`, porque nenhum chamador chega a ser autenticado.

```console
$ curl -s -o /dev/null -w '%{http_code}\n' -XPOST localhost:8080/v1/polls/vote \
    -H 'content-type: application/json' -d '{"choice_id":1}'
403
```

Uma rota *pública* (um método de fachada sem `#[requires]`) serve qualquer um. Se
quisesses que o índice de perguntas fosse legível por todos, tirarias o
`#[requires]` de `list`.

## Admitir chamadores com um authenticator

Aponta o servidor para o JWKS do teu provider OIDC; ele constrói um
`JwtAuthenticator` e monta-o à frente da verificação de permissão de cada rota:

```console
$ export SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
$ export SEIVA_OIDC_ISSUER=https://id.example.com/
$ export SEIVA_OIDC_AUDIENCE=polls-site
$ seiva start
```

Agora o comportamento é:

| Pedido | Resultado |
| --- | --- |
| sem bearer | `anonymous` — rotas públicas servem, rotas guardadas `403` |
| bearer inválido | `401` + `WWW-Authenticate` |
| bearer válido | `Principal` verificado; a permissão `polls.*` da rota é verificada |

Por isso um token a transportar `polls.vote` pode votar mas não criar perguntas; um
token com `polls.manage` pode fazer ambas. Os tokens são verificados
criptograficamente (apenas RS256/ES256, nunca `alg:none`; `iss`/`aud`/`exp`
validados). As permissões vêm de uma claim configurável. Ver
[Autenticação](../../topics/authentication.md).

## Autorização ao nível do objecto

A permissão ao nível da função (`#[requires]`) diz *quem pode chamar*; a autorização
ao nível do objecto diz *que linhas podem tocar* — para um serviço polls
multi-inquilino, que perguntas de qual inquilino um chamador pode ver. O `Scoped<T>`
do Seiva torna o acesso entre inquilinos/entre donos uma preocupação de tempo de
compilação, fail-closed — ver [Autorização](../../topics/authorization.md) e
[Segurança](../../topics/security.md).

## A seguir

[Parte 5: Testar o teu módulo](part-5-testing.md).

---

*Parte da [documentação do Seiva](../../index.md).*

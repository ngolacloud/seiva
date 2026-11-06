# Parte 7: De monólito a microserviço

As Partes 1–6 construíram a polls como um **monólito modular** — um binário,
`apps/polls-site-api`, com o módulo `polls` delimitado lá dentro. Esse é o default
certo: uma só coisa para construir, implantar e raciocinar. Mas uma fronteira de
módulo no Seiva é uma costura *real*, e quando um módulo precisa de escalar,
implantar ou falhar por si só, podes promovê-lo a **microserviço** — sem tocar no
código do módulo.

Este é o retorno da fronteira de cinco camadas que tens respeitado desde o início:
o módulo `polls` já não depende de nada que não devesse, por isso pô-lo de pé
sozinho é um passo de scaffolding, não uma reescrita.

## Separar o módulo

```console
$ seiva module split polls
Split module `polls` into a service. Created apps/polls-service (mounts only `polls`), registered it in the workspace and set mode = "service".
Run the microservice: `cargo run -p polls-service` (or `dx run -p polls-service`). The monolith still serves `polls` too; `seiva module merge polls` undoes this.
```

`seiva module split` faz três coisas, todas elas edições legíveis que podes ler no
diff:

1. **Gera `apps/polls-service/`** — um segundo composition root (o seu próprio
   `main`) que monta **apenas** o módulo `polls`, sobre o mesmo serve loop de
   runtime que o monólito. Recebe as rotas da framework à borla: `/health`,
   `/openapi.json`, e a documentação interactiva (`/docs`, `/redoc`, `/scalar`).
2. **Regista-o** como membro do workspace.
3. **Regista `mode = "service"`** em `modules/polls/module.toml`.

Não move **nenhum código**. Os cinco crates de `modules/polls/` ficam intactos, e o
monólito continua a servir `polls` como antes — passas simplesmente a ter uma
segunda forma de correr o mesmo módulo.

## Correr o microserviço

=== "dx"

    ```console
    $ dx run -p polls-service
    ```

=== "cargo"

    ```console
    $ cargo run -p polls-service
    ```

É um serviço Seiva completo que por acaso hospeda um módulo. Aponta-o a uma porta e
serve as rotas `polls` e o seu próprio OpenAPI:

```console
$ SEIVA_HTTP_ADDR=127.0.0.1:8081 cargo run -p polls-service &
$ curl -s localhost:8081/openapi.json | grep -o '"/v1/polls"'
"/v1/polls"
$ curl -s -o /dev/null -w '%{http_code}\n' localhost:8081/docs    # Swagger UI
200
```

O contrato do serviço também é imprimível offline, tal como o do monólito:

```console
$ SEIVA_EMIT=openapi cargo run -p polls-service    # just the polls facade
```

## O que corre onde

| | Monólito `apps/polls-site-api` | Microserviço `apps/polls-service` |
| --- | --- | --- |
| Módulos servidos | todos os módulos do projecto | apenas `polls` |
| Unidade de deploy | um binário/imagem | o seu próprio binário/imagem (o seu próprio `Delonixfile`) |
| Escala | como um todo | de forma independente |
| Código do módulo | `modules/polls/*` | os **mesmos** `modules/polls/*` |

Como ambos os roots ligam os crates de módulo idênticos, não há uma segunda
implementação para manter em sincronia — o contrato, os casos de uso e os testes
são partilhados. Escolhes a forma do deploy; o módulo não muda.

## Voltar atrás

Mudaste de ideias, ou o tráfego assentou? Dobra-o de volta no monólito:

```console
$ seiva module merge polls
Merged `polls` back into the monolith: removed apps/polls-service and reset mode = "library". The module still serves inside apps/<project>-api.
```

`merge` remove `apps/polls-service`, descarta o seu membro de workspace e repõe o
mode. O módulo `polls` em si fica, de novo, intacto.

## gRPC

Um serviço separado é também onde o gRPC ganha o seu lugar: a mesma fachada já gera
um documento de serviço proto3 (`seiva proto`), por isso um microserviço `polls`
pode expor gRPC ao lado do HTTP para chamadas serviço-a-serviço. Ver
[Documentos de API](../../topics/api-documents.md#proto3-seiva-proto).

## O fim — e o início

Construíste uma aplicação polls completa (dados, validação, autenticação,
resultados, testada, empacotada), depois implantaste-a de duas formas a partir de
uma só base de código. Para onde a seguir:

- [Parte 8: um frontend no browser com dxjs](part-8-a-dxjs-frontend.md) — põe uma
  UI reactiva segura em CSP sobre a API polls e torna-a full-stack.
- Os [guias temáticos](../../topics/index.md) para profundidade em cada camada.
- Os how-tos [Adicionar um módulo](../../howto/add-a-module.md) e
  [Trocar para o DORM](../../howto/switch-to-dorm.md).
- A [referência da CLI](../../reference/cli.md) para o tour completo de comandos.

---

*Parte da [documentação do Seiva](../../index.md).*

# Parte 5: Testar o teu módulo

Uma aplicação polls merece testes. O módulo `polls` traz-nos, e esta parte mostra
os três níveis que o Seiva torna fáceis: testes unitários de caso de uso, as
funções de fitness da arquitectura, e como os correr.

## Teste unitário ao caso de uso (sem servidor, sem BD)

A camada de aplicação programa contra portas, por isso o caso de uso testa-se com
fakes em memória de `seiva-testing` — sem HTTP, sem base de dados. Este é o teste
real de
[`examples/polls/application`](https://github.com/ngolacloud/seiva/tree/main/examples/polls/application),
condensado:

```rust
#[tokio::test]
async fn ask_add_choice_vote_and_read_back() {
    let svc = service();                                   // fakes for every port

    let q = svc.ask(CreateQuestion { text: "What's new?".into() }).await.unwrap();
    let a = svc.add_choice(AddChoice { question_id: q.id, text: "Not much".into() }).await.unwrap();

    svc.vote(Vote { choice_id: a.id }).await.unwrap();
    let after = svc.vote(Vote { choice_id: a.id }).await.unwrap();
    assert_eq!(after.votes, 2);                            // the tally is real

    let detail = svc.detail(GetQuestion { id: q.id }).await.unwrap();
    assert_eq!(detail.choices[0].votes, 2);
}
```

A suite prova também os caminhos infelizes: uma pergunta vazia é recusada pela
regra do domínio (nada faz commit), uma opção para uma pergunta inexistente dá erro,
e votar numa opção desconhecida **reverte a transacção** e não emite nenhum evento.
Corre-os:

```console
$ cargo test
```

## Funções de fitness da arquitectura

`seiva lint` corre as invariantes que mantêm a arquitectura honesta — por exemplo,
a camada `interface` não pode depender de `infrastructure`, e a `application` de um
módulo não pode nomear `delonix_seiva` directamente:

```console
$ seiva lint
```

Estas falham o build quando uma regra de camadas é quebrada, por isso o desenho de
cinco camadas não pode erodir (ver o catálogo `seiva-pattern-fitness`).

## Testes de controlador / HTTP

O próprio `seiva-controller-test` da framework prova que um `#[seiva::controller]`
gera um router funcional, guardado e versionado de ponta a ponta — o padrão a
copiar quando quiseres testar as rotas da polls com um cliente de teste (um `403`
sem token, `201` para um `ask` válido, a contagem de votos a subir).

## O princípio

Põe a validação de entrada em `Validate`, as regras de negócio no `domain` (a
contagem de votos, o texto não vazio — testados em unidade sem dependências), e
mantém a camada de aplicação livre de transporte para que os seus testes precisem
apenas de fakes. As coisas caras (uma base de dados, um servidor) são para os
poucos testes de integração, não para os muitos testes unitários. Ver
[Testes](../../topics/testing.md).

## A seguir

[Parte 6: Empacotamento e deploy](part-6-packaging-and-deploy.md).

---

*Parte da [documentação do Seiva](../../index.md).*

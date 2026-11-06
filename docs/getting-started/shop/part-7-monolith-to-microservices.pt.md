# Parte 7: Do monólito aos microserviços

A loja foi entregue como um binário. Agora duas preocupações precisam de sair por
conta própria: **payments** tem de correr num deployment isolado (os seus próprios
segredos, o seu próprio raio de impacto), e **notifications** faz trabalho lento e
em rajadas que deveria escalar de forma independente. Na maioria dos stacks isso é
uma reescrita. No Seiva é uma mudança de composição — o código do módulo não se
move.

## Separar um módulo no seu próprio serviço

```console
$ seiva module split payments
$ seiva module split notifications
```

`seiva module split` promove um módulo a serviço autónomo: ganha o seu próprio
composition root (`apps/payments-api`), a sua própria imagem e o seu próprio
deployment, enquanto os crates **exactamente iguais** de `modules/payments/`
compilam para dentro dele. O monólito restante (`catalog`, `inventory`, `orders`)
continua a correr como um binário.

Nada dentro de `modules/payments/` mudou — nem a fachada, nem a regra de domínio,
nem o caso de uso. Esse é todo o propósito de codificar a camada de aplicação
contra portas e manter os módulos independentes (Partes 2–3): *onde* um módulo
corre é uma decisão de deployment, tomada por último.

## Os eventos agora atravessam a rede

No monólito, `orders.placed` alcançava a reacção de pagamento sobre um bus em
processo. Com `payments` separado, o *mesmo* evento atravessa agora um **tópico de
broker** — e porque a publicação já aconteceu dentro da transacção da encomenda e
aterrou no outbox (Parte 4), o worker de relay encaminha-o para o broker
exactamente uma vez. O código da reacção é idêntico; só o transporte por baixo dele
mudou.

```text
monolith:      orders ──(in-process bus)──▶ payments.charge
microservices: orders ──(outbox → broker topic)──▶ payments service ──▶ payments.charge
```

A permissão `payments.charge` (Parte 5) faz agora também trabalho real na fronteira
do serviço: o serviço de pagamento aceita a reacção apenas de um chamador que
possua essa capacidade, por isso uma separação não alarga a superfície de ataque.

## Quando separar — e quando não

Começa como um monólito. Um deploy, uma transacção, um único sítio onde olhar — até
um módulo ter uma *razão* para se mover: isolamento (payments), escalamento
independente (notifications), uma equipa separada, um runtime diferente. O Seiva
permite-te adiar essa decisão porque a fronteira é real desde o primeiro dia;
separar é barato precisamente porque não tiveste de o prever.

## Construíste uma loja

De `seiva project new` a uma loja de cinco módulos que corre como um monólito
modular e depois como uma frota de serviços — o mesmo código, um botão de
deployment. Para onde a seguir:

- Os [guias por tópico](../../topics/index.md) para profundidade em cada camada.
- A [secção DORM](../../dorm/index.md) e a [secção dx](../../dx/index.md).
- O [panorama competitivo](../../competitive-landscape.md) para onde o Seiva se
  posiciona face a outros stacks.

---

*Parte da [documentação do Seiva](../../index.md).*

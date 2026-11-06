# Parte 9: Do monólito aos microserviços

A loja foi entregue como um binário. Agora duas preocupações precisam de sair por
conta própria: **payments** tem de correr num deployment isolado (os seus próprios
segredos, o seu próprio raio de impacto), e **notifications** faz trabalho lento e
em rajadas que deveria escalar independentemente. Na maioria das stacks isso é uma
reescrita. No Seiva é uma mudança de composição — o código do módulo não se move.

## Separar um módulo no seu próprio serviço

```console
$ seiva module split payments
$ seiva module split notifications
```

`seiva module split` promove um módulo a serviço autónomo: ganha a sua própria raiz
de composição (`apps/payments-api`), a sua própria imagem e o seu próprio
deployment, enquanto os **exactamente mesmos** crates `modules/payments/` compilam
para dentro dele. O monólito restante (`catalog`, `inventory`, `orders`) continua a
correr como um binário.

Nada dentro de `modules/payments/` mudou — nem a fachada, nem a regra de domínio,
nem o caso de uso. É esse todo o objectivo de programar a camada de aplicação
contra portas e manter os módulos independentes (Partes 2–3): *onde* um módulo
corre é uma decisão de deployment, tomada por último.

## Os eventos agora atravessam a rede

No monólito, `orders.placed` chegava à reacção de pagamento sobre um barramento em
processo. Com `payments` separado, o *mesmo* evento atravessa um **tópico de
broker** em vez disso — e porque a publicação já aconteceu dentro da transacção da
encomenda e aterrou na outbox (Parte 4), o trabalhador de relay encaminha-o para o
broker exactamente uma vez. O código da reacção é idêntico; só mudou o transporte
por baixo dele.

```text
monolith:      orders ──(in-process bus)──▶ payments.charge
microservices: orders ──(outbox → broker topic)──▶ payments service ──▶ payments.charge
```

A permissão `payments.charge` (Parte 5) agora também faz trabalho real na fronteira
do serviço: o serviço de pagamentos aceita a reacção apenas de um chamador que
possua essa capacidade, por isso uma separação não alarga a superfície de ataque.

## Quando separar — e quando não

Começa como um monólito. Um deploy, uma transacção, um lugar para olhar — até um
módulo ter uma *razão* para se mover: isolamento (payments), escalonamento
independente (notifications), uma equipa separada, um runtime diferente. O Seiva
deixa-te adiar essa decisão porque a fronteira é real desde o primeiro dia; separar
é barato precisamente porque não tiveste de o prever.

A loja agora corre como um monólito modular e, onde isso compensa, como uma frota
de serviços — o mesmo código, um botão de deployment. Restam duas partes: dar ao
back office uma UI sem código de frontend, e uma SPA virada ao cliente na sua
própria origem.

## Seguinte

[Parte 10: Uma consola admin, sem frontend escrito](part-10-admin-console.md).

---

*Parte da [documentação do Seiva](../../index.md).*

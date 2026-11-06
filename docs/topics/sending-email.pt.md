# Envio de email

> **Estado: uma porta planeada.** Esta página descreve o formato pretendido; o
> email ainda não é uma porta de primeira classe no framework.

O email transaccional (uma mensagem de boas-vindas, uma ligação de reposição de
palavra-passe, um recibo) é um **efeito secundário de saída** — e o Seiva já tem o
padrão certo para esses.

## O formato pretendido: uma porta de saída + a outbox

O email deve ser uma **porta** da camada de aplicação (um trait que a aplicação
invoca, p. ex. `Mailer::send`), implementada por um adaptador em `infrastructure`.
Para fiabilidade, um email transaccional pertence à **outbox transaccional**, como
os eventos de domínio: enfileira o "enviar email de boas-vindas" na mesma
transacção que a criação de utilizador que o segue, e deixa um worker entregá-lo —
para que um email nunca seja enviado por uma alteração que foi revertida, e nunca
seja perdido se o fornecedor estiver brevemente indisponível. Ver
[mensageria e eventos](messaging-and-events.md) e
[workers em segundo plano](background-workers.md).

## Hoje

Até existir uma porta `Mailer`, envia email a partir de um adaptador em
`infrastructure` que o teu módulo injecta (um cliente SMTP ou de API de
fornecedor), idealmente despoletado a partir do relay da outbox em vez de inline no
pedido. Trata as credenciais do fornecedor como [configuração](configuration.md)
(ambiente, nunca embutidas).

## Relacionado

- [Mensageria e eventos](messaging-and-events.md) · [Workers em segundo plano](background-workers.md)

---

*Parte da [documentação do Seiva](../index.md).*

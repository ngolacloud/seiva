# ADR 0001 — Fundações do Delonix Seiva

- Estado: Aceite
- Data: 2026-11-03
- Decisores: NgolaCloud

## Contexto

O Delonix Seiva é um framework de backend em Rust para sistemas robustos e de
alta criticidade: só backend, ergonómico e com baterias incluídas, um servidor
pronto para produção e para cluster, e o DORM (o ORM da Delonix) como camada de
dados por omissão. É um produto novo no seu próprio repositório, distinto do
DORM, que se mantém um ORM independente (ADR 0073 do DORM).

Três decisões fundacionais têm de ficar fixadas antes de se construir qualquer
módulo ou runtime, porque toda a escolha posterior depende delas: onde vive o
código, onde se origina o contrato de serviço, e qual é o barramento de
mensagens por omissão.

## Decisão

### 1. Repositório

O Seiva vive no seu próprio repositório, `ngolacloud/seiva`, crate
`delonix-seiva`, CLI `seiva`. Não é um workspace dentro do DORM.

O Seiva depende do DORM; o DORM nunca do Seiva. Enquanto o DORM não publicar uma
versão com a API de biblioteca estável, o Seiva depende do DORM **por git com
uma revisão fixada** (nunca por caminho local), apenas através do gateway
`seiva-data-dorm`. Assim que o DORM publicar a API de biblioteca, o Seiva passa
a depender das crates publicadas, e os dois produtos mantêm-se compatíveis por
uma matriz de compatibilidade documentada.

O núcleo do framework não depende do DORM. O ORM entra apenas através do gateway
`seiva-data-dorm` por trás da feature `dorm` (activa por omissão). Um projecto
cujos módulos usem todos uma camada de dados externa não liga ORM nenhum.

### 2. Origem do contrato — Rust primeiro

O contrato público de um módulo origina-se em Rust. O atributo
`#[seiva::facade]` sobre um trait é a fonte única de verdade; a partir dele o
Seiva gera o `.proto` versionado e revisível, o servidor e o cliente gRPC, o
documento OpenAPI e os testes de contrato. O `.proto` gerado é committado e
revisível, para que uma alteração de contrato fique visível na revisão.

Um módulo também pode partir de um `.proto` existente ao integrar com um sistema
que já é dono do contrato; nesse caso a fachada Rust gerada leva o cabeçalho
"generated, do not edit" e o `.proto` é a fonte.

### 3. Barramento de mensagens por omissão — NATS JetStream

O broker por omissão é o NATS JetStream: é operacionalmente leve, tem suporte
Rust de primeira classe e dá streams duráveis com confirmação pelo consumidor,
que é o que o relay do outbox e o inbox precisam. O Kafka está disponível por
trás de uma feature para implantações que já o correm ou precisam do seu débito
e retenção. O AMQP (RabbitMQ) mantém-se também disponível por trás de uma
feature.

O broker é por módulo: um módulo declara o seu barramento, e o relay do outbox
desse módulo aponta para ele. Os clientes de mensagens vêm de `seiva-messaging`
(o código que sai do `dorm-messaging` do DORM, com o histórico preservado).

## Consequências

- O workspace fundacional leva três crates: `seiva-ports` (fronteira só-traits,
  estável em SemVer por política), `delonix-seiva` (o ponto de entrada do
  framework) e `seiva-cli` (o binário `seiva`). Nenhuma delas depende do DORM; a
  feature `dorm` é declarada em `delonix-seiva` mas não arrasta nada enquanto o
  gateway `seiva-data-dorm` não existir.
- A fronteira entre o DORM e o Seiva é governada pela regra da "fronteira do
  ORM" (fixada numa ADR do DORM na Parte DORM-C): uma capacidade entra no DORM
  só se fizer sentido para quem usa o DORM sem framework nenhum. Tudo cuja
  garantia venha da comunicação entre serviços, do tempo, da identidade do
  chamador ou de política é do Seiva e vive por trás de uma porta.
- Contratos Rust-primeiro significam que os artefactos gRPC/OpenAPI/`.proto` são
  gerados e nunca editados à mão (Generation Gap); o lint falha se o forem.

## Alternativas consideradas

- **Seiva como workspace dentro do DORM.** Rejeitada: acoplaria as duas cadências
  de release e baralharia a dependência de sentido único. O DORM tem de se manter
  utilizável sem framework.
- **Contratos Protobuf-primeiro.** Rejeitada como omissão: empurra os DTOs e a
  validação para fora do sistema de tipos do Rust, perdendo a propriedade "os
  tipos são a fonte de tudo". Permitida como opção para integrações cujo dono é o
  `.proto`.
- **Kafka como barramento por omissão.** Rejeitada como omissão: mais pesado de
  operar para o caso comum. Mantido por trás de uma feature.

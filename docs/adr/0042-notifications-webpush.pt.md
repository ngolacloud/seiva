# ADR 0042 — Notificações nativas: um hub multi-canal com in-app, Web Push e email, alimentado pelo barramento de eventos

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0021 (barramento de eventos in-process e coreografia), ADR 0031 (real-time nativo), ADR 0034 (fila de trabalhos durável + leases), ADR 0041 (mail nativo)

## Contexto

«Algo aconteceu — avisa o utilizador.» Essa frase repete-se em toda a plataforma:
chega um convite, uma reunião está prestes a começar, faz-se uma menção, um export
fica pronto. As peças para a entregar existem já no Seiva separadas — o barramento
de eventos (ADR 0021) diz *o que aconteceu*, o real-time (ADR 0031) pode empurrar
in-app, o mail (ADR 0041) pode enviar email — mas nada as amarra numa única
**notificação** com um store, preferências por utilizador, e entrega por vários
canais. O delonix-meet constrói isto à mão por funcionalidade, pelo que um «foste
convidado» é cablado de três maneiras diferentes. O hub que faz fan-out de um
evento de domínio para os canais certos, respeitando as escolhas do utilizador, é
horizontal e pertence ao framework.

As forças:

- **Uma notificação, muitos canais.** Uma notificação é um registo de 1ª classe
  (tipo, actor, assunto, payload, lida/não-lida), não um push efémero. É guardada
  para a inbox in-app **e** entregue out-of-band aos canais em que o utilizador fez
  opt-in: in-app (via ADR 0031), Web Push, email (via ADR 0041). O registo é a
  fonte da verdade; os canais são entregas dele.
- **Alimentada pelo barramento, não pelos call-sites.** Um módulo que coloca uma
  encomenda não devia saber como as notificações são entregues. Publica o seu
  evento de domínio (ADR 0021); um subscritor de notificações mapeia o evento para
  uma notificação e faz-lhe fan-out. Isto mantém o produtor ignorante da entrega —
  o princípio da coreografia.
- **As preferências do utilizador são política, a entrega é mecanismo.** *Se* um
  utilizador quer email para menções é escolha do utilizador; *como* um email é
  enviado é trabalho do framework. O hub guarda preferências por utilizador por
  tipo-de-notificação e honra-as; o produto define os tipos e os seus defaults.
- **O Web Push é um protocolo, não uma vibe.** Tem uma spec real: uma subscrição
  (`endpoint` + chaves `p256dh` + `auth`) obtida no browser, um payload **cifrado**
  para essas chaves (`aes128gcm`, RFC 8291), e um JWT **VAPID** (RFC 8292) assinado
  com a chave do servidor a identificar o servidor da aplicação. O framework tem de
  implementar isto correctamente, uma vez — uma cifra feita à mão é um bug de
  segurança à espera de acontecer.
- **O fan-out é fora do request.** Cifrar um payload por subscrição e fazer POST a
  cada push service é I/O que não pode bloquear o produtor do evento. Corre na fila
  de trabalhos (ADR 0034), em batch, com retry.

## Decisão

Adicionar um novo membro do workspace **`seiva-notifications`** — sem camada de
dados (um port `NotificationStore`), atrás de feature, a orquestrar as primitivas
existentes.

- **O modelo de notificação** — `Notification` (id, tenant, destinatário, tipo,
  actor, assunto, `data`, `created_at`, `read_at`) atrás de um port
  `NotificationStore` (double in-memory; adapter DORM/sqlx por ADR 0030). O store é
  a inbox in-app: listar, marcar-lida, contagem de não-lidas, tudo
  **tenant-scoped**.
- **O pipeline de fan-out** — um subscritor de `seiva-events` (ADR 0021) recebe um
  evento de domínio, um mapper registado pelo produto transforma-o numa
  `Notification`, o store persiste-a, e o hub enfileira um job de entrega **por
  cada canal com opt-in** no `seiva-jobs` (ADR 0034). Os produtores ficam
  ignorantes dos canais.
- **Canais, cada um um adapter:**
  - **In-app** — publica a notificação guardada no `Topic` de real-time do
    destinatário (ADR 0031), para que um cliente aberto a veja ao vivo; o store
    cobre os clientes offline.
  - **Web Push** — um `PushSubscriptionStore` guarda `{endpoint, p256dh, auth}` por
    utilizador/dispositivo, tenant-scoped. A entrega **cifra** o payload com
    `aes128gcm`/RFC 8291 para as chaves da subscrição e assina um JWT **VAPID**
    (RFC 8292). Um `410 Gone`/`404` do push service **purga** a subscrição morta.
  - **Email** — renderiza e enfileira via `seiva-mail` (ADR 0041).
- **Preferências** — um store `NotificationPreferences`, por utilizador por tipo
  por canal, com defaults definidos pelo produto; o fan-out consulta-o antes de
  enfileirar. Um canal sem subscrição simplesmente não é enfileirado.
- **Política que o produto mantém** — os *tipos* de notificação, os seus templates,
  as suas preferências default, e os mappers evento→notificação. O framework
  carrega o store, o pipeline, a cripto de Web Push, e os adapters de canal.

Todos os POSTs de Web Push de saída passam pelo egress/SSRF guard (ADR 0045). O
`tokio` entra com `default-features = false`; os canais Web Push e email são
features opcionais, para que um serviço só com inbox in-app não ligue nenhum.

## Alternativas consideradas

- **Empurrar directamente de cada call-site por real-time.** Rejeitado: acopla
  cada produtor à entrega, salta a inbox guardada (utilizadores offline perdem o
  evento), e re-implementa as preferências por toda a parte. O hub alimentado pelo
  barramento com um store é o propósito.
- **Um único canal «notify» (só in-app).** Rejeitado: os utilizadores esperam
  email e browser push para coisas que não estão a ver. Multi-canal com
  preferências por utilizador é a expectativa de base.
- **Fazer à mão a cifra de Web Push por produto.** Rejeitado em absoluto: o
  `aes128gcm` da RFC 8291 e o VAPID são fáceis de errar subtilmente, e uma cifra de
  payload errada é um defeito de segurança. Implementá-la uma vez, revista uma vez.
- **Tratar uma notificação como um evento fire-and-forget.** Rejeitado: uma
  notificação é um registo durável, lida/não-lida, ao qual o utilizador regressa;
  modelá-la como evento perde a inbox e o estado-de-leitura.

## Consequências

- **Performance.** O fan-out é **assíncrono** na fila de trabalhos, para que o
  produtor do evento nunca espere por I/O de push/email; as entregas são em batch;
  o canal in-app anda sobre o hub de real-time existente a custo marginal quase
  nulo; os canais Web Push e email são features opcionais.
- **Segurança.** Os payloads de Web Push são **cifrados** (`aes128gcm` da RFC 8291)
  e autenticados com **VAPID** (RFC 8292); subscrições e notificações são
  **tenant-scoped**; os canais são estritamente **opt-in** por utilizador; os POSTs
  de saída passam pelo egress/SSRF guard (ADR 0045); nenhum payload é registado.
- **Resiliência.** A entrega é um job durável (ADR 0034) com retry de backoff
  limitado; um `410 Gone` de um push service **purga** a subscrição expirada para
  que endpoints mortos se auto-curem; multi-canal significa que a indisponibilidade
  de um canal ainda alcança o utilizador por outro e sempre pela inbox guardada.
- O delonix-meet colapsa a sua ligação de notificações por-funcionalidade em
  mappers de evento + preferências, e ganha browser push de graça; toda a
  plataforma futura ganha um hub de notificações ao depender de um crate.

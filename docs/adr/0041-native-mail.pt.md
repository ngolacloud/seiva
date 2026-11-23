# ADR 0041 — Mail nativo: um port `Mailer` com um adapter SMTP e envio assíncrono

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0002 (core ports e doubles in-memory), ADR 0034 (fila de trabalhos durável + leases)

## Contexto

Quase toda a plataforma tem de enviar email transaccional: verificar um endereço,
repor uma palavra-passe, aceitar um convite, notificar um actor. O Seiva hoje não
tem capacidade de mail nenhuma — por isso o único consumidor que dela precisa, o
delonix-meet, liga o `lettre` à mão, constrói os seus próprios templates, e envia
**inline dentro do request**, o que acopla uma latência voltada ao utilizador a um
relay SMTP de terceiros que pode estar lento ou em baixo. É exactamente a forma que
devia ser uma primitiva do framework: um port contra o qual a aplicação programa,
um adapter SMTP atrás de feature, e um envio que nunca bloqueia o request.

As forças:

- **Em forma de port, com o core sem adapter.** A aplicação tem de depender de um
  trait `Mailer` (como todo o outro port, ADR 0002), nunca do `lettre`
  directamente. O fio SMTP, o provider, as credenciais são questão do adapter,
  composto na raiz. Um serviço que não envia mail não liga stack SMTP nenhuma e
  nada paga.
- **Enviar não é trabalho de request.** Um handshake SMTP pela rede pode levar
  segundos e pode falhar transitoriamente. Fazê-lo dentro do request amarra um
  login ou um registo à saúde de um relay. O envio tem de ser **enfileirado** e
  correr fora do request, na fila de trabalhos durável (ADR 0034), para que o
  request regresse de imediato e a entrega volte a tentar no seu próprio calendário.
- **O conteúdo é política, o mecanismo é horizontal.** A *forma* de enviar —
  endereçamento, cabeçalhos, MIME, TLS, templating, retry — é verdade para
  qualquer plataforma. O *texto* do email «verifique o seu endereço», as
  identidades de remetente, a marca são política do produto. O framework carrega o
  primeiro e injecta o segundo.
- **A injecção de cabeçalhos é um ataque real.** Um `name` ou `subject` construído
  a partir de input do utilizador que contenha `\r\n` pode injectar cabeçalhos
  extra (um `Bcc:` escondido, um `From:` forjado). O adapter tem de rejeitar
  caracteres de controlo em cada campo ligado a cabeçalho antes de a mensagem ser
  sequer construída.
- **A entregabilidade e a segurança do transporte não são opcionais.** O TLS para
  o relay é obrigatório (STARTTLS ou implícito), e o produto precisa de consciência
  de DKIM/SPF/DMARC — no mínimo uma identidade de assinatura configurada e
  pré-verificações DNS/MX opcionais — ou o seu mail cai em spam. O framework expõe
  isto como config, não como surpresa.

## Decisão

Adicionar um novo membro do workspace **`seiva-mail`** — sem camada de dados,
atrás de feature, a depender do port `Mailer` e do `seiva-jobs` (ADR 0034) para o
envio assíncrono.

- **`Mailer`** — o port: `async fn send(&self, message: &Mail) -> MailResult<()>`.
  `Mail` é um valor validado (envelope `from`/`to`/`cc`/`bcc`, `subject`, um corpo
  texto **e** HTML, anexos opcionais, cabeçalhos custom). A construção **rejeita
  CR/LF e caracteres de controlo** em cada campo ligado a cabeçalho, para que um
  payload de injecção de cabeçalhos nunca se torne mensagem.
- **Adapters, compostos na raiz:**
  - **`SmtpMailer`** (feature `lettre`) — um transporte SMTP com pool. **TLS é
    obrigatório**: TLS implícito ou STARTTLS, nunca auth em texto-claro;
    autenticação de relay (user/password ou token) de config 12-factor. Um
    **connection pool** limitado amortiza o handshake pelos envios.
  - **`ConsoleMailer`** / **`MemoryMailer`** — os doubles de dev/teste:
    renderizam a mensagem para stdout ou capturam-na para asserções, para que
    nenhum módulo precise de um relay vivo para ser exercitado (a regra dos doubles
    do ADR 0002).
- **Templating** — um pequeno registo de templates propriedade do produto
  (verificação, recuperação, convite…) a renderizar texto+HTML a partir de
  contexto tipado. O motor é o mecanismo; os templates e o texto são política do
  produto, substituíveis por tenant. A saída renderizada é HTML-escaped por
  omissão.
- **Envio assíncrono via `seiva-jobs`** — a aplicação chama um fino
  `enqueue_mail(message)` que persiste um job `SendMail` e regressa de imediato.
  Um worker reclama-o, renderiza, e chama o `Mailer`; uma falha transitória volta a
  tentar com backoff limitado e aterra na dead-letter queue após o tecto (ADR
  0034). Um `send` síncrono está disponível para o raro chamador que tenha de
  bloquear.
- **Consciência de entregabilidade** — config para a identidade de assinatura
  (selector DKIM), o remetente de envelope, e **pré-verificações DNS/MX opcionais**
  antes do enqueue (um domínio de destinatário mau falha cedo em vez de moer
  retries). Lookups de MX de saída e qualquer webhook/callback passam pelo egress
  guard (ADR 0045), nunca um socket cru.
- **Rate limiting** — um orçamento de envio por tenant (reusando
  `seiva-ratelimit`, ADR 0015) tecta um chamador comprometido ou em ciclo antes de
  queimar a reputação do relay.

O `tokio` entra com `default-features = false`; nenhum runtime é ligado à
biblioteca. O `lettre` e as verificações DNS são features opcionais.

## Alternativas consideradas

- **Chamar o `lettre` directamente de cada módulo.** Rejeitado: solda a aplicação
  a um transporte, volta a derivar a segurança de cabeçalhos e o templating em cada
  produto, e — pior — envia inline, acoplando a latência do request ao relay. O
  port + envio assíncrono é o propósito todo.
- **Enviar sincronamente dentro do request.** Rejeitado: uma paragem SMTP torna-se
  uma paragem visível ao utilizador e uma falha transitória de relay torna-se um
  email perdido. Enqueue + retry na fila durável é estritamente melhor; um caminho
  síncrono fica para o raro chamador que tenha de bloquear.
- **Entregar uma DSL de templating completa no framework.** Rejeitado: texto,
  layout e marca são política. Um registo fino sobre um motor padrão mantém a linha
  mecanismo-vs-política; o produto é dono dos templates.
- **Um SDK de provider alojado (SES/SendGrid) como core.** Rejeitado como default:
  o SMTP é o chão soberano, neutro quanto a provider. Um adapter de provider é uma
  impl `Mailer` *adicional* legítima, não a base.

## Consequências

- **Performance.** O envio corre **fora do request** na fila de trabalhos, para
  que um login ou registo nunca espere pelo SMTP; um transporte SMTP com pool
  amortiza o handshake TLS pelas mensagens; o crate não liga runtime e é zero-cost
  quando um serviço não envia mail.
- **Segurança.** O TLS para o relay é obrigatório (sem auth em texto-claro); cada
  campo ligado a cabeçalho rejeita CR/LF, fechando a injecção de cabeçalhos; os
  corpos são HTML-escaped; um rate limit por tenant tecta o abuso; lookups de MX e
  callbacks passam pelo egress/SSRF guard (ADR 0045); as credenciais vêm de config
  e nunca são registadas.
- **Resiliência.** A entrega é um job durável (ADR 0034) com retry de backoff
  limitado e uma dead-letter queue, para que uma indisponibilidade transitória de
  relay seja sobrevivida, não perdida; timeouts de envio tectam um handshake
  parado; um relay secundário pode ser configurado como fallback de provider.
- O delonix-meet apaga a sua ligação `lettre` feita à mão e os envios inline,
  mantendo só o seu texto de email e identidades de remetente como política; toda a
  plataforma futura ganha mail transaccional ao depender de um crate.

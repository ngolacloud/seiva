# ADR 0045 — Um cliente HTTP de saída guardado: protecção de egress / SSRF para webhooks, mail e OIDC

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0012 (saúde e limites cloud-native), ADR 0026 (controlo de carga por rota), ADR 0033 (motor de entrega de webhooks)

## Contexto

Uma plataforma faz chamadas HTTP de saída para **endereços que não controla**: o
URL de webhook de um tenant (ADR 0033), o host MX do domínio de um destinatário
(ADR 0041), os endpoints de discovery e JWKS OIDC de um provider de identidade (ADR
0040), uma funcionalidade «vai buscar este URL» fornecida pelo utilizador. Cada uma
dessas é um vector de **Server-Side Request Forgery**: um tenant que regista um
webhook a apontar para `http://169.254.169.254/` ou `http://10.0.0.5:6379/` está a
pedir ao *servidor* que alcance o serviço de metadados da cloud ou um Redis interno
por ele — de dentro da fronteira de confiança, com a identidade de rede do servidor.
O Seiva hoje tem só uma menção de passagem a validação de egress; vários caminhos de
saída novos (0033, 0040, 0041, 0042) tornam um guard único, correcto e partilhado um
pré-requisito, não um extra.

As forças:

- **Validar o IP resolvido, não o hostname.** Bloquear por string («negar
  `localhost`») é trivialmente contornável: `127.0.0.1.nip.io`, um IP decimal, um
  hostname que *resolve* para um endereço privado. O guard tem de resolver o DNS e
  inspeccionar o **IP real** contra os ranges bloqueados — loopback, privado
  (`10/8`, `172.16/12`, `192.168/16`), link-local (`169.254/16`, o IP de metadados),
  `fc00::/7`, `::1`, e os restantes.
- **O DNS rebinding é o ataque subtil.** O domínio de um atacante pode passar a
  validação (resolve para um IP público), e depois resolver para `169.254.169.254`
  na *segunda* consulta que o cliente HTTP faz quando efectivamente liga. O guard tem
  de **fixar o IP que validou** e ligar-se a esse endereço exacto, para que o IP que
  foi verificado seja o IP que é marcado — sem segunda resolução não-verificada.
- **Os redirects reabrem o buraco.** Um URL público e permitido pode fazer `302`
  para `http://10.0.0.5/`. Seguir redirects às cegas derrota a verificação toda. Cada
  salto tem de ser **re-validado** com as mesmas regras, e a contagem de redirects
  limitada.
- **Um guard, todos os chamadores.** Webhooks, MX do mail, fetch OIDC e qualquer
  fetch do produto têm de passar todos pelo *mesmo* cliente verificado, ou o
  call-site não-guardado mais fraco é o buraco de SSRF da plataforma. Um único
  cliente partilhado e guardado é o mecanismo; as allowlists por-produto são
  política.
- **Guardado ≠ sem limites.** Uma chamada de saída é também uma superfície de
  resiliência: precisa de timeouts de connect/read (a postura do ADR 0012/0026,
  aplicada à saída) e de um circuit breaker por host para que uma dependência morta
  não paralise cada worker.

## Decisão

Adicionar um **cliente HTTP de saída guardado** — uma pequena superfície
`seiva-egress` a embrulhar o `reqwest`, partilhada por cada caminho de saída — que
valida endereços antes e ao longo do request inteiro.

- **`EgressPolicy`** — as regras: os ranges de IP bloqueados (loopback, privado,
  link-local, unique-local, o metadados `169.254.169.254`/`fd00:ec2::254`), uma
  **allowlist** por-produto opcional (só estes hosts/ranges) e **denylist**, os
  esquemas permitidos (`https` por omissão), o máximo de redirects, e os timeouts. A
  política é injectada; o mecanismo é fixo.
- **Marcação resolve-and-pin.** Para cada conexão o guard resolve o host, rejeita o
  request se **algum** IP resolvido estiver num range bloqueado, depois **fixa** o IP
  validado escolhido e liga-se a ele directamente — o IP que foi verificado é o IP
  que é usado, fechando o **DNS rebinding**. Uma cache curta de resolução de DNS
  evita re-resolver no caminho quente sem alargar a janela de rebinding para além do
  seu TTL.
- **Re-validar ao longo dos redirects.** O seguimento automático de redirects é
  substituído por um ciclo manual que corre a **mesma** validação resolve-and-pin em
  **cada** salto e impõe o tecto de redirects; um salto para um range bloqueado
  aborta o request inteiro. Os cabeçalhos de credenciais/authorization **não** são
  encaminhados por um redirect cross-origin.
- **Timeouts e circuit breaking.** Timeouts de connect e read limitam cada chamada
  (ADR 0012/0026 aplicado à direcção de saída); um **circuit breaker por host**
  (reusando `seiva-resilience`, ADR 0033) dispara um destino repetidamente falhado
  para que deixe de consumir workers, e recupera numa sonda half-open.
- **O cliente partilhado, injectado.** O `seiva-webhooks` (0033), o `seiva-mail`
  (0041), o Web Push do `seiva-notifications` (0042), e o fetch de discovery/JWKS
  OIDC (0040) recebem todos este cliente em vez de um `reqwest::Client` cru. Uma
  funcionalidade de fetch própria do produto também o usa. Não há caminho suportado
  para um cliente de saída não-guardado no framework.

O `reqwest`/DNS vivem atrás da feature do crate; um serviço que não faz chamadas de
saída não liga nada disto.

## Alternativas consideradas

- **Validar só a string do hostname/URL.** Rejeitado: trivialmente contornável por
  um nome que resolve para um IP privado, por codificações de IP alternativas, ou por
  um redirect. Só inspeccionar o **IP resolvido**, em cada salto, é sólido.
- **Validar uma vez, depois deixar o `reqwest` resolver e ligar.** Rejeitado: isso
  deixa uma janela de **DNS rebinding** — a resolução do próprio cliente pode diferir
  da que foi verificada. Fixar o IP validado é a correcção.
- **Seguir redirects com a política built-in do `reqwest`.** Rejeitado: o seguidor
  built-in não re-corre uma verificação SSRF por salto, pelo que um `302` para um
  endereço interno é seguido. É preciso um ciclo manual re-validante.
- **Um guard por consumidor (cada crate faz o seu).** Rejeitado: o SSRF é uma
  propriedade de plataforma inteira; o único call-site que esquece a verificação é a
  brecha. Um único cliente partilhado e revisto remove essa classe de erro.
- **Confiar antes no firewalling de egress de rede (NetworkPolicy).** Rejeitado como
  controlo único: a política de rede é uma camada *adicional* valiosa mas é dependente
  da implantação e ausente em dev; a aplicação tem de ser segura por si
  (defense-in-depth), e um bloqueio de `169.254.169.254` pertence ao cliente de
  qualquer forma.

## Consequências

- **Performance.** Uma cache curta de resolução de DNS e um cliente reutilizado com
  pool mantêm o guard fora do custo do caminho quente; a validação é um punhado de
  comparações de range por conexão; o crate não liga nada quando um serviço não faz
  chamadas de saída.
- **Segurança.** IPs internos e de **metadados** (`169.254.169.254`, link-local,
  privado, loopback, unique-local) são bloqueados (**SSRF**); o IP validado é
  **fixado** contra **DNS rebinding**; cada salto de **redirect** é re-validado e
  limitado; credenciais internas **não** são encaminhadas por um redirect
  cross-origin; só `https` por omissão.
- **Resiliência.** **Timeouts** de connect/read limitam cada chamada de saída (ADR
  0012/0026); um **circuit breaker por host** (ADR 0033, `seiva-resilience`) isola um
  destino falhado para que uma dependência má não esgote o pool de workers; o tecto
  de redirects limita um ciclo de redirects.
- Cada caminho de saída no Seiva — webhooks, mail, notificações, OIDC, fetch do
  produto — torna-se seguro-quanto-a-SSRF por construção através de um cliente
  injectado, e o delonix-meet reforma as suas verificações ad-hoc por um único guard
  revisto.

# ADR 0036 — `seiva-auth-mfa`: segundo factor TOTP com backup codes e step-up

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade e autenticação), ADR 0015 (rate limit distribuído), ADR 0017 (hashing de palavra-passe plugável)

## Contexto

O Seiva autentica um chamador com **um** factor: um JWT verificado (ADR 0006) ou
uma palavra-passe através do fluxo de referência `authenticate_password` (ADR
0017). Um único factor significa que uma palavra-passe vazada ou obtida por
phishing é uma tomada de conta completa — o controlo que todo o comprador
empresarial, e todo o regime de conformidade, pede primeiro é um **segundo factor**.
O *driver*, o delonix-meet, constrói à mão a inscrição e a verificação TOTP; é
mecanismo horizontal puro (o RFC 6238 é o mesmo para qualquer produto) e pertence
ao framework.

O TOTP (uma one-time password baseada no tempo, o que uma app autenticadora mostra)
é o primeiro segundo-factor certo: sem custo de SMS nem exposição a SIM-swap, sem
dependência de hardware, funciona offline, e interopera com todas as apps
autenticadoras via um único provisioning URI `otpauth://`. Mas carrega arestas
afiadas que um framework tem de acertar uma vez, não deixar cada equipa
redescobrir.

As forças:

- **O segredo partilhado é uma credencial.** O segredo TOTP é tão sensível como uma
  palavra-passe — quem o detiver pode cunhar códigos válidos para sempre. Tem de
  estar **cifrado em repouso**, nunca registado, e nunca devolvido depois da
  inscrição.
- **O desvio de relógio é real, o replay é a ameaça.** Um verificador tem de aceitar
  uma pequena janela de time-steps adjacentes (os telefones derivam), mas um código
  é só one-time: o **mesmo** código não pode ser aceite duas vezes (de outro modo um
  atacante que espreite um código tem ~30 s). O último contador aceite tem de ser
  lembrado.
- **Os códigos são adivinháveis em escala.** Seis dígitos são uma adivinha em um
  milhão; sem um throttle um atacante simplesmente tenta. A verificação de MFA tem
  de ficar atrás do mesmo rate limit do login (ADR 0015).
- **Perder o telefone não pode trancar a conta.** A inscrição tem de cunhar
  **backup / recovery codes** — one-time, hasheados em repouso como as palavras-passe
  (ADR 0017) — para que um dispositivo perdido seja uma recuperação, não um ticket
  de suporte a caminho da eliminação da conta.
- **Algumas acções merecem um desafio fresco.** Manter uma sessão não é
  consentimento para apagar a org. O **step-up** redesafia o segundo factor para uma
  acção sensível mesmo dentro de uma sessão viva.

## Decisão

Adicionar um novo membro do workspace **`seiva-auth-mfa`** — sem camada de dados,
sem transporte, compondo com o `seiva-identity` e o `seiva-auth`.

- **Inscrição** — `begin_enrolment(subject)` sorteia um segredo TOTP aleatório
  (≥ 160 bits, `OsRng`) e devolve o **provisioning URI** de uma só vez
  (`otpauth://totp/{issuer}:{account}?secret=...&issuer=...`) para a app mostrar
  como QR. `confirm_enrolment(code)` exige um código válido antes de o factor ser
  armado (provando que o segredo foi guardado correctamente), e só então persiste o
  segredo **cifrado**.
- **Verificação** — `verify(subject, code, now)` computa o HMAC do RFC 6238 sobre o
  time-step actual e uma allowlist de ±N steps adjacentes (`skew_window`, default
  ±1), e **rejeita qualquer step igual ou abaixo do último contador aceite**
  (anti-replay). Um acerto avança o contador guardado.
- **`MfaSecretStore`** / **`BackupCodeStore`** (ports) — guardam o segredo cifrado,
  o último contador aceite, e os backup codes hasheados. O `seiva-testing` duplica-os;
  um adapter sobre DORM vive atrás da feature `dorm`.
- **Backup codes** — `issue_backup_codes(n)` devolve `n` códigos de alta entropia
  **uma vez** e guarda só os seus hashes (via os `PasswordHashers` do `seiva-auth`,
  reusando o ADR 0017). `redeem_backup_code(code)` é one-time: um código resgatado é
  marcado como gasto.
- **Step-up** — um `StepUpChallenge { subject, action, expires_at }` que o produto
  cunha antes de um método de facade sensível; `verify_step_up` consome-o com um TOTP
  fresco ou um backup code, devolvendo uma prova de curta duração que a rota verifica.
- **`MfaPolicy`** — por org: que rotas/acções exigem MFA, se é obrigatório em toda a
  org, a janela de skew, e o TTL do step-up. *Que* acções são sensíveis é política do
  produto; o mecanismo é do framework.

A verificação é HMAC barato; o throttle é o rate limiter do `seiva-http` indexado
pelo subject (ADR 0015). O crate não liga runtime e é zero-cost quando não usado.

## Alternativas consideradas

- **Códigos one-time por SMS / email.** Rejeitado como primeiro factor: o SMS
  carrega risco de SIM-swap e intercepção e um custo por mensagem; o email é tão
  fraco como a conta de email. O TOTP é offline, grátis, e resistente a replay em
  massa por phishing. (Um canal posterior pode implementar o mesmo port.)
- **Sem anti-replay, aceitar qualquer código na janela.** Rejeitado: um código
  espreitado por cima do ombro fica válido durante todo o step. Lembrar o último
  contador aceite é o que faz de uma one-time password de facto one-time.
- **Guardar o segredo TOTP em plaintext.** Rejeitado: uma fuga da base de dados
  deixaria um atacante cunhar códigos válidos indefinidamente — o segredo é uma
  credencial e está cifrado em repouso, como os hashes de palavra-passe ao seu lado.
- **Sem backup codes (reinscrever ao perder o dispositivo).** Rejeitado: um telefone
  perdido significaria um fluxo de suporte com prova de identidade ou a perda da
  conta. Recovery codes hasheados e one-time são a saída de emergência padrão, de
  autosserviço.
- **Pôr o TOTP dentro do `seiva-identity`.** Rejeitado: o MFA é uma capacidade
  distinta e opcional com os seus próprios stores e dependências (HMAC, QR). Um crate
  separado mantém o `seiva-identity` enxuto e deixa o MFA evoluir de forma
  independente, espelhando como o real-time (ADR 0031) se separou do `seiva-http`.

## Consequências

- **Performance.** A verificação é um punhado de computações **HMAC** sobre uma
  janela de steps minúscula — microssegundos, sem I/O além de uma leitura do store. O
  crate não liga runtime e é zero-cost para um serviço que não inscreve MFA.
- **Segurança.** O segredo TOTP está **cifrado em repouso** e nunca é devolvido
  depois da inscrição; o **anti-replay** (janela de skew + último contador aceite)
  faz de cada código verdadeiramente one-time; a verificação fica atrás do **rate
  limit** (ADR 0015) para que os códigos não sejam atacados por força bruta; os
  backup codes são **hasheados, one-time** (ADR 0017); e o **step-up** reprova o
  factor para acções sensíveis dentro de uma sessão viva — defesa em profundidade,
  fail-closed.
- **Resiliência.** Os **recovery codes** fazem de um dispositivo perdido um
  autosserviço, não um incidente de perda de conta; os stores de segredo/contador/backup
  são **ports**, por isso um só processo e uma implantação multi-pod partilham uma
  lógica com adapters diferentes; uma inscrição confirmar-antes-de-armar evita
  trancar um utilizador fora com um segredo mal guardado.
- O delonix-meet apaga o seu código TOTP feito à mão e guarda só a sua política —
  que acções exigem MFA, se as suas orgs o impõem — como configuração.
- O crate é o primeiro de uma família de factores de identidade: o factor
  passkey/WebAuthn (ADR 0037) compõe-se com ele (o TOTP é o fallback quando uma
  passkey não está disponível), atrás do mesmo `Principal` e da mesma costura de
  step-up.

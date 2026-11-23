# ADR 0037 — `seiva-auth-webauthn`: passkeys / FIDO2 via `webauthn-rs`

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade e autenticação), ADR 0036 (MFA / TOTP)

## Contexto

O TOTP (ADR 0036) eleva a fasquia, mas continua a ser um **segredo partilhado
digitado por um humano**: uma página de phishing convincente pede os seis dígitos e
o atacante repete-os em tempo real. O único factor que fecha o phishing em tempo
real é a autenticação de **chave pública, vinculada à origem** — WebAuthn / FIDO2,
apresentada aos utilizadores como **passkeys**. A chave privada nunca sai do
autenticador (TPM da plataforma, Face/Touch ID, ou uma chave de segurança), a
asserção é assinada sobre um **challenge** emitido pelo servidor, e o browser
vincula-a à origem do site real — um site de phishing simplesmente não consegue
produzir uma asserção válida. É para aqui que a história de identidade empresarial
caminha, e o *driver*, o delonix-meet, vai precisar dela; a mecânica das cerimónias
é 100 % horizontal.

As forças:

- **Não fazer CBOR/COSE/attestation à mão.** O WebAuthn é um protocolo binário denso
  (objectos de attestation em CBOR, chaves públicas COSE, CTAP). Interpretá-lo à mão
  é exactamente o campo minado criptográfico que o ADR 0006 recusou para os JWTs —
  delegamos no auditado e maduro **`webauthn-rs`**.
- **O challenge é one-time e de curta duração.** Toda a cerimónia gira sobre um
  challenge aleatório fresco cunhado pelo servidor; a asserção assinada só é válida
  para *aquele* challenge, uma vez, antes de um TTL curto expirar. Repetir uma
  asserção capturada tem de falhar porque o challenge já está gasto.
- **O binding de Origin / RP-ID é a propriedade anti-phishing.** A asserção carrega a
  origem com que o browser de facto falou; o verificador tem de fixar o `RP-ID` e a
  origem esperados, para que uma credencial registada para o site real não possa ser
  usada a partir de um sósia. Esta é a propriedade que falta ao TOTP.
- **Um autenticador clonado tem de ser detectável.** Cada credencial carrega um
  **contador de assinatura** monotonicamente crescente; se uma asserção apresentar um
  contador igual ou abaixo do valor guardado, o autenticador pode ter sido clonado e
  a tentativa tem de ser recusada.
- **Tem de coexistir com os outros factores.** Uma passkey nem sempre está disponível
  (um dispositivo emprestado, uma chave perdida). O WebAuthn tem de compor-se com o
  TOTP (ADR 0036) como fallback, atrás do mesmo `Principal` e costura de step-up —
  nunca um beco sem saída.

## Decisão

Adicionar um novo membro do workspace **`seiva-auth-webauthn`** — sem camada de
dados, sem transporte, envolvendo o **`webauthn-rs`** e compondo com o
`seiva-identity`.

- **Cerimónia de registo** — `begin_registration(subject)` pede ao `webauthn-rs` uma
  `CreationChallengeResponse`, guarda o estado de registo correspondente no **challenge
  store** sob um TTL curto, e devolve o challenge ao browser. `finish_registration(subject,
  credential)` verifica a attestation e persiste uma `StoredCredential { credential_id,
  public_key, sign_count, aaguid, created_at }`.
- **Cerimónia de autenticação** — `begin_authentication(subject)` cunha uma
  `RequestChallengeResponse` (de novo guardada com TTL); `finish_authentication(assertion)`
  verifica a assinatura contra a chave pública guardada, verifica origin/RP-ID, e —
  criticamente — verifica que o **sign-count avançou**; um contador não-crescente é um
  possível clone e é **recusado**. Um sucesso actualiza o contador guardado e rende um
  `Principal` (ou uma prova de step-up, ADR 0036).
- **`ChallengeStore`** (port) — guarda o estado de cerimónia em curso indexado por um
  nonce, cada um com um TTL; um challenge é **consumido uma vez** (removido no finish)
  para não poder ser repetido. O `seiva-testing` duplica-o; um adapter Redis serve o
  multi-pod (o browser pode terminar num pod diferente daquele onde começou).
- **`CredentialStore`** (port) — as credenciais registadas por subject (`credential_id`,
  `public_key`, `sign_count`, `aaguid`). O `seiva-testing` duplica-o; um adapter sobre
  DORM vive atrás da feature `dorm`.
- **`WebAuthnPolicy`** — por org: que utilizadores podem/devem inscrever uma passkey, a
  exigência de **user verification** (presença vs. verificado), se as **resident keys**
  (passkeys descobríveis, login sem username) são permitidas, e o transporte de
  **attestation** (nenhum por default; directo só onde a proveniência do dispositivo é
  exigida).

A verificação de assinatura é cripto assimétrica barata; nenhum runtime é ligado à
biblioteca; o adapter Redis do challenge store é uma feature opcional.

## Alternativas consideradas

- **Fazer o WebAuthn à mão (interpretar CBOR/COSE directamente).** Rejeitado pela mesma
  razão que o ADR 0006 recusou a verificação de JWT à mão: a superfície de
  attestation/COSE é um campo minado de footguns. O `webauthn-rs` é auditado e segue a
  especificação.
- **Só TOTP, sem passkeys.** Rejeitado: o TOTP é passível de phishing em tempo real (o
  utilizador lê o código para uma página falsa). Só a auth de chave pública vinculada à
  origem fecha isso, e é a direcção para onde a identidade empresarial se move.
  Entregamos **ambos** e deixamos o produto escolher por política.
- **Ignorar o contador de assinatura.** Rejeitado: o contador é o detector de clones
  embutido do WebAuthn; largá-lo deita fora um sinal de integridade grátis. Um contador
  que regride bloqueia a tentativa.
- **Guardar o challenge num cookie assinado em vez de um store do servidor.** Rejeitado:
  um store do lado do servidor com um TTL dá consumo verdadeiramente one-time e funciona
  entre pods; um cookie é repetível até expirar e complica o finish multi-pod.
- **Dobrar o WebAuthn no `seiva-auth-mfa`.** Rejeitado: o WebAuthn arrasta uma
  dependência pesada e distinta (`webauthn-rs`) e um modelo de cerimónia/credencial
  diferente. Um crate irmão mantém as dependências e a superfície de cada factor
  isoladas, ambos atrás do único `Principal`.

## Consequências

- **Performance.** Cada cerimónia é uma **verificação de assinatura assimétrica** mais
  dois toques pequenos no store (cunhar challenge, consumir + actualizar contador) —
  cripto barata, I/O limitado. O crate não liga runtime e é zero-cost quando um serviço
  não inscreve passkeys.
- **Segurança.** O challenge é **one-time com um TTL** (uma asserção capturada está
  gasta), o verificador **fixa o Origin / RP-ID** (resistente a phishing por construção —
  a propriedade que falta ao TOTP), um **sign-count que regride bloqueia** a tentativa
  (detecção de clonagem), a attestation está disponível onde a proveniência do
  dispositivo importa, e a chave privada nunca sai do autenticador. Fail-closed em toda
  a parte, consistente com o ADR 0019.
- **Resiliência.** O **challenge store é um port com um TTL**, por isso uma cerimónia
  abandonada autoexpira e uma implantação multi-pod pode terminar onde não começou
  (adapter Redis); e o WebAuthn **recai sobre outro factor** — o TOTP (ADR 0036) —
  quando uma passkey não está disponível, por isso um dispositivo perdido nunca é uma
  exclusão. Ambos os stores são ports, partilhados entre processo único e multi-pod com
  adapters diferentes.
- O delonix-meet apaga a sua canalização de cerimónias WebAuthn e guarda só a sua
  política — que utilizadores inscrevem, a sua postura de attestation, a sua exigência
  de user-verification — como configuração.
- Juntos, os ADRs 0035–0037 fazem do Seiva um fornecedor de identidade de primeira
  parte completo (emitir + rodar sessões, TOTP, passkeys) atrás do único `Principal` do
  ADR 0006, não apenas um *verificador* de tokens.

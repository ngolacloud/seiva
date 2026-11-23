# ADR 0035 — Refresh tokens rotativos e revogáveis com detecção de reutilização

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0006 (identidade e autenticação), ADR 0019 (autorização fail-closed)

## Contexto

O Seiva **verifica** tokens, não os **emite**. O `JwtAuthenticator` do
`seiva-identity` transforma um bearer num `Principal` só a partir de um OIDC/JWT
criptograficamente verificado (RS256/ES256, `iss`/`aud`/`exp` validados), e o ADR
0006 mantém a *emissão* de tokens como «trabalho de um IdP». Isso está certo
quando existe um IdP externo — mas uma plataforma auto-hospedada como o *driver*,
o delonix-meet, **é** o seu próprio IdP: autentica um utilizador com uma palavra-passe
(ADR 0017) e tem depois de entregar uma sessão que sobrevive mais do que um único
access token de curta duração sem voltar a pedir a palavra-passe.

A resposta ingénua — um único JWT de vida longa — é a armadilha. Um JWT sem estado
não pode ser revogado antes de expirar, por isso um TTL longo faz de um token
roubado uma credencial válida durante dias sem interruptor do lado do servidor, e
um TTL curto obriga o utilizador a reautenticar-se constantemente. A resposta da
indústria é a **divisão em dois tokens**: um access token de curta duração
(minutos, sem estado, exactamente o que o `JwtAuthenticator` já verifica) mais um
**refresh token** de vida longa (dias) revogável do lado do servidor e trocado por
um access token fresco. O delonix-meet constrói isto à mão hoje; é 100 %
horizontal e pertence ao framework.

As forças:

- **Revogável por construção.** O valor do refresh token é que o logout, uma
  mudança de palavra-passe, ou um «matar todas as sessões» de administrador de
  facto *terminam* o acesso. Isso exige estado do lado do servidor que o access
  token deliberadamente não tem.
- **Nunca confiar no token em repouso.** O store guarda um **hash** do refresh
  token, nunca o plaintext — uma base de dados vazada não pode render sessões
  utilizáveis, tal como as palavras-passe nunca são guardadas em claro (ADR 0017).
- **Rotação one-time-use.** Cada troca cunha um novo refresh token e reforma o
  antigo. Um refresh token é um cupão de uso único, não uma chave reutilizável, o
  que encolhe a janela em que um token capturado serve a um só pedido.
- **Reutilização é roubo.** A rotação torna os tokens roubados *detectáveis*: se um
  token já rodado é apresentado de novo, ou o cliente legítimo ou um atacante está
  a repetir um token gasto — toda a **família** (sessão) está comprometida e tem de
  ser revogada de uma vez (o padrão OAuth de refresh-token-reuse).
- **Store plugável, fail-closed.** O store é um port, como qualquer outro backend
  (ADR 0015/0030); qualquer lookup ambíguo nega (ADR 0019), nunca concede.

## Decisão

Adicionar a emissão de refresh tokens ao **`seiva-identity`** (sem camada de
dados), compondo com o `Authenticator`/`JwtAuthenticator` existentes em vez de os
substituir.

- **`RefreshToken`** — um segredo opaco de alta entropia (≥ 256 bits de `OsRng`),
  devolvido ao cliente **uma vez** no momento da cunhagem. O seu registo do lado do
  servidor carrega um `family_id`, um `token_hash`, o `subject`/`tenant` que vincula,
  `issued_at`, `expires_at`, um binding opcional a `device`/`session`, e um marcador
  `rotated_at`/usado. O plaintext nunca é persistido.
- **`RefreshTokenStore`** (port) — `insert`, `find_by_hash(hash) -> Option<Record>`,
  `mark_rotated(id, successor_id)`, `revoke_family(family_id)`. O lookup é **por
  hash** sobre uma coluna indexada (SHA-256 do segredo apresentado), por isso a
  verificação é uma leitura indexada, não uma varredura de tabela. O `seiva-testing`
  entrega um duplo em memória; um adapter sobre DORM vive atrás da feature `dorm`.
- **`issue(subject, tenant, policy)`** — abre um novo `family_id`, cunha o primeiro
  refresh token, e emparelha-o com um access JWT de curta duração.
- **`rotate(presented)`** — a troca one-time-use: fazer o hash do segredo
  apresentado, procurá-lo, e
  - **não encontrado / expirado** → rejeitar (`401`), sem mudança de estado;
  - **encontrado mas já rodado** → **reutilização detectada**: `revoke_family` a
    sessão inteira e rejeitar — este é o sinal de roubo;
  - **encontrado e vivo** → marcá-lo como rodado, cunhar um sucessor na *mesma*
    família, e devolver um novo par access + refresh.
- **`revoke(family_id)`** / **`revoke_all(subject)`** — logout e «matar as minhas
  sessões»; idempotente (revogar uma família já revogada é um no-op com sucesso).
- **`RefreshPolicy`** — botões por org: TTL do access token (minutos), TTL do
  refresh (dias), um tecto de vida absoluta da sessão, e se a rotação é obrigatória.
  A política de revogação (ex.: revogar-tudo-ao-mudar-palavra-passe) é do produto,
  cablada via o barramento de eventos (ADR 0021).

O `tokio`/cripto entram com features mínimas; nenhum runtime é ligado à biblioteca.
Os adapters do store são features opcionais; só-local em memória é o default.

## Alternativas consideradas

- **Um único JWT de vida longa, sem refresh.** Rejeitado: um JWT sem estado não
  pode ser revogado antes do `exp`, por isso um token roubado é válido durante toda
  a sua vida sem interruptor do servidor — exactamente a falha que um login de
  plataforma não pode entregar.
- **Só um cookie de sessão do lado do servidor (sessão opaca com estado).** É
  viável, mas descarta a verificação sem estado e sem cache do access JWT em cada
  pedido. A divisão em dois tokens mantém a auth do caminho quente sem estado
  (verificar o JWT) e paga o custo do store só no momento do refresh.
- **Guardar o refresh token em plaintext (ou cifrado reversivelmente).** Rejeitado:
  uma fuga da base de dados entregaria sessões vivas. Hash-em-repouso + lookup-por-hash
  dá o mesmo match O(1) sem um segredo recuperável, espelhando o armazenamento de
  palavras-passe.
- **Rodar mas não detectar reutilização.** Rejeitado: a rotação sozinha estreita a
  janela mas deita fora o seu melhor sinal. Tratar um token gasto repetido como
  roubo e revogar a família é o que transforma a rotação de higiene em detector.
- **Emitir tokens num crate novo.** Rejeitado: emissão e verificação são duas
  metades de uma só história de identidade; mantê-las no `seiva-identity` deixa o
  `issue` reusar directamente o assinador JWT e o modelo `Principal`.

## Consequências

- **Performance.** A verificação permanece sem estado e sem cache (o access JWT é
  verificado exactamente como hoje); o store só é tocado em `rotate`/`revoke`, e aí
  como uma única leitura **indexada por hash**. O crate não liga runtime e é
  zero-cost para um serviço que usa um IdP externo e nunca emite.
- **Segurança.** Os refresh tokens são **hasheados em repouso** (uma fuga da BD não
  rende sessão utilizável), **one-time-use** (um token capturado serve a um só
  pedido), e **detectam reutilização** (um token gasto repetido revoga a família
  inteira — a resposta ao roubo). A revogação é do lado do servidor e real
  (logout/mudança-de-palavra-passe/kill de administrador terminam de facto o
  acesso), e todo o lookup ambíguo nega — fail-closed, consistente com o ADR 0019.
- **Resiliência.** A revogação é **idempotente** (revogar uma família já morta tem
  sucesso), por isso retries e caminhos concorrentes de logout/kill convergem; o
  store é um **port**, por isso um só processo (em memória) e uma implantação
  multi-pod (backend partilhado) usam a mesma lógica com um adapter diferente; um
  tecto de sessão absoluto limita quanto tempo qualquer família pode viver
  independentemente do tráfego de refresh.
- O delonix-meet apaga a sua canalização de refresh/rotação/reutilização feita à
  mão e guarda só a sua política — que rotas cunham uma sessão, os seus TTLs por
  org, e os eventos de domínio que disparam o `revoke_all` — como configuração do
  produto.
- A *verificação* de tokens contra um IdP externo (ADR 0006) fica inalterada; isto
  acrescenta o lado **emissor** para plataformas que são o seu próprio IdP, atrás
  do mesmo `Principal`.

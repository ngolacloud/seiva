# ADR 0049 — `seiva-kms`: cifra por envelope com a chave num guardião, e segredos presos ao seu contexto

- Status: Aceite
- Data: 2026-11-27
- Decisores: NgolaCloud
- Segue: ADR 0002 (portos do núcleo e duplos de teste), ADR 0012 (sondas de saúde cloud-native), ADR 0047 (TLS mútuo, o precedente de uma capacidade partilhada que passa para o Seiva)

## Contexto

Os serviços Seiva guardam segredos próprios: sementes TOTP (ADR 0036), client
secrets OAuth, palavras-passe de bind LDAP, credenciais SSH e WinRM, a chave
privada de uma CA interna, segredos de assinatura de webhooks. Sem uma resposta
comum, cada projecto cifra-os à sua maneira, quase sempre com uma chave numa
variável de ambiente. Essa chave não roda sem deixar ilegíveis todos os segredos
guardados, e uma cópia de segurança da base de dados vale tanto como o texto em
claro para quem também tiver o ambiente.

Dois produtos construídos sobre o Seiva já deram com isto:

- O **dxstorage** resolveu-o (F8 `4e23bc7`, F11 `fb98cf6`, cerca de 2 000 linhas
  com testes): um porto `Wrapper` sobre o motor transit do HashiCorp Vault e os
  KMS da AWS, da Google Cloud e da Azure; um anel de chaves de dados (a actual
  sela, as anteriores só abrem), rotação com re-selagem e retirada; segredos
  selados como `kms1:<id-da-chave>:<base64>` com AES-256-GCM. Está acoplado à
  aplicação (a configuração do seu motor, `anyhow`, o armazenamento do cluster),
  por isso mais ninguém pode depender dele, e não prende dados associados: um
  blob selado copiado para outra linha ou outra conta continua a abrir.
- O **Device Sentinel** precisa já de um cofre de segredos (sementes TOTP, a
  chave da CA dos agentes, a semente de assinatura dos comandos; mais tarde as
  credenciais da automação e da sonda), com um anel por organização, para a sua
  V1.

O dono do projecto decidiu a 2026-11-27 que o código do dxstorage passa para
**um crate partilhado no Seiva** em vez de ser copiado, o mesmo caminho que o TLS
mútuo seguiu (ADR 0047): os dois produtos já dependem do Seiva.

As forças:

- **A chave de cifra de chaves (KEK) nunca sai do seu guardião.** No Vault é a
  chave do transit; sem Vault existe só na memória do processo, lida de um
  ficheiro montado. As chaves de dados só se guardam embrulhadas.
- **Um segredo está preso ao seu contexto.** Um blob movido para outro registo,
  outro propósito ou outra organização não pode abrir. É a melhoria em relação ao
  dxstorage.
- **Rodar sem parar**, tanto a KEK (no guardião) como as chaves de dados (na
  aplicação), com os segredos re-selados em fundo.
- **Falha fechada.** Um anel corrompido, uma chave que não desembrulhou, um
  guardião em baixo no arranque: um erro, nunca uma chave fixa nem texto em claro.
- **Sem opinião sobre armazenamento.** O dxstorage guarda o anel no
  armazenamento do cluster, o Sentinel no Postgres. O crate não pode escolher.
- **Sem DORM, e o build por omissão continua sem cliente HTTP.**

## Decisão

Acrescentar ao workspace o crate **`seiva-kms`**.

**O porto do guardião, `KeyWrapper`** (o `Wrapper` do dxstorage com um tipo de
erro próprio em vez de `anyhow`): `provider`, `kek_id`, `generate` (uma chave de
dados nova e a sua forma embrulhada), `unwrap`, `rewrap` (para a versão mais nova
da KEK, `None` quando já é a actual) e `health`. Os erros são um `KmsError` cuja
variante `Unavailable` (inacessível, 5xx, 429, selado) é a única que vale a pena
repetir; `Refused` (autenticação, permissão, chave desconhecida) é final.

**Dois adaptadores na V1, cada um atrás de uma funcionalidade do Cargo:**

- **`local`** (por omissão): o `LocalKek` lê um ficheiro com uma chave de 32
  bytes em base64 por linha. A primeira embrulha; as outras só desembrulham. Uma
  chave de dados embrulhada é `local:<impressão-da-kek>:<base64>`
  (XChaCha20-Poly1305), por isso, depois de se pôr uma chave nova no topo do
  ficheiro, o carregamento seguinte volta a embrulhar o anel com ela e a linha
  antiga pode sair.
- **`vault`**: o `VaultTransit` sobre reqwest + rustls (fornecedor ring), com
  autenticação por token, ficheiro de token (relido a cada chamada), **AppRole** e
  **Kubernetes**; uma CA própria; um namespace; uma espera de arranque
  `wait_ready(timeout)`. Um token de login fica guardado durante metade do seu
  lease e é substituído (um login novo, uma nova tentativa) quando o Vault
  responde 403. Os redireccionamentos não são seguidos e o cabeçalho do token é
  marcado como sensível. A saúde é o `sys/health` (standbys aceites) mais o
  `auth/token/lookup-self`.

O AWS KMS, o Google Cloud KMS e o Azure Key Vault são portados do dxstorage mais
tarde, atrás do mesmo porto.

**O anel.** O `WrappedRing` é a forma guardada: uma versão do formato, a
`WrappedKey` actual e as anteriores (id, guardião, nome da KEK, forma embrulhada,
data de criação), e `rotated_at`; serializa-se em JSON e é validado na leitura
(formato conhecido, ids válidos e únicos, nada vazio). O `KeyRing` é a forma
desembrulhada. É **puro** (sem I/O, sem relógio): `seal(aad, plain)` e
`open(aad, sealed)`, `rotated`, `retired`, `with_rewrapped`, `needs_reseal`. Só
se constrói quando todas as chaves do anel estão desembrulhadas; uma chave em
falta é um erro, nunca um anel parcial. As chaves são apagadas da memória quando
largadas e nunca se imprimem.

**O formato selado: `sv1:<id-da-chave>:<base64(nonce ‖ cifra ‖ etiqueta)>`**, com
**XChaCha20-Poly1305**, um nonce aleatório de 24 bytes e **dados associados**. O
`Aad::new(org, purpose, record)` constrói `<org>|<propósito>|<registo>` (a
organização e o propósito não podem conter `|`, para a divisão ser inequívoca); o
`Aad::raw` aceita quaisquer bytes não vazios. Dados associados vazios são
recusados: um segredo está sempre preso a alguma coisa. Um blob que não
autentica devolve `KmsError::Decrypt`, que não diz se o errado foi a chave, os
dados associados ou os bytes.

**Compatibilidade com o dxstorage, só de leitura.** O `KeyRing::open_kms1` abre
os blobs `kms1:` do dxstorage (AES-256-GCM, sem dados associados), e o
`WrappedRing::from_dxstorage_json` lê o anel que ele guarda, com os seus ids de
chave, para o dxstorage poder adoptar o crate e re-selar aos poucos. O `open`
recusa o `kms1:`, para que quem chama nunca aceite um blob solto onde espera um
preso. O `with_open_only` acrescenta chaves fora do anel que só abrem (as chaves
estáticas do dxstorage).

**Os portos de quem usa.** O `RingStore` guarda um anel embrulhado por *âmbito*
(uma organização, uma instalação) com escritas **compare-and-swap** (o `load`
devolve o anel e uma versão; o `store` só escreve se a versão não mudou). Um anel
que não se lê é um erro do `load`, nunca "ausente", para que um anel corrompido
nunca seja substituído por um novo que deixaria órfãos todos os segredos. O
`Reseal` re-sela os segredos de um âmbito com um dado `KeyRing`. O
`MemoryRingStore` vem incluído para os testes.

**O ciclo de vida, `RingKeeper`**, sem estado, sobre um `KeyWrapper` e um
`RingStore`:

- `load(scope)`: cria o anel na primeira utilização (um escritor ganha a
  corrida, os outros lêem o anel dele), senão verifica que foi feito por este
  guardião e esta KEK, desembrulha cada chave e volta a embrulhar as que estão
  numa versão antiga da KEK (guardado sem garantia);
- `boot(scope, timeout)`: o `load` repetido perante `Unavailable`, com espera
  crescente (0,5 s a duplicar até 8 s) até ao prazo, e depois o erro: o serviço
  não fica `ready`;
- `rotate(scope, resealers)`: uma chave de dados nova passa a selar, a actual
  junta-se às anteriores, guardado por compare-and-swap (`Conflict` se outra
  réplica chegou primeiro), e depois correm todos os re-seladores; as falhas deles
  são relatadas, não fatais;
- `rotate_if_older` para uma agenda;
- `retire(scope, after, resealers)`: passado `after` desde a rotação, todos os
  re-seladores têm de ter sucesso, e só então as chaves anteriores saem.

Guardar o `KeyRing` em memória, relê-lo por notificação ou por temporizador, e os
bloqueios entre réplicas ficam com quem usa.

## Alternativas consideradas

- **Copiar o código do dxstorage para o Sentinel.** Recusado pelo dono do
  projecto: duas linhagens do mesmo código de segurança afastam-se, e as
  correcções chegam só a uma.
- **Um repositório próprio para o crate.** Recusado: os dois utilizadores já
  dependem do Seiva, e os gates do CI do Seiva (fmt, clippy, deny, docs)
  aplicam-se de graça.
- **AES-256-GCM no formato novo, como o dxstorage.** Recusado: um nonce aleatório
  de 12 bytes limita quantas mensagens uma chave pode selar antes de as colisões
  se tornarem um risco. O nonce de 24 bytes do XChaCha20-Poly1305 tira esse limite
  na prática, e é Rust puro (RustCrypto) sem dependência de hardware. O Sentinel
  já usa ChaCha20-Poly1305.
- **Aceitar o `kms1:` no `open`.** Recusado: um atacante que consiga escrever
  numa coluna poderia plantar um blob solto que abre seja qual for o registo. Ler
  o `kms1:` é uma chamada explícita de migração.
- **O crate ser dono do armazenamento (uma tabela Postgres, um ficheiro).**
  Recusado: os dois utilizadores guardam os anéis em sítios completamente
  diferentes; o compare-and-swap é a única propriedade de que o crate precisa,
  por isso é ele o porto.
- **Estado da rotação dentro do crate (uma tarefa de fundo, uma cache).**
  Recusado por agora: o mecanismo de notificação (o `NOTIFY` do Postgres, a
  difusão entre pares do dxstorage) é da aplicação. O `RingKeeper` fica sem
  estado.
- **O cliente do Vault directamente sobre hyper.** Recusado: o reqwest é o que os
  dois utilizadores e o `seiva-llm-reqwest` já usam; mantê-lo atrás da
  funcionalidade `vault` deixa o build por omissão sem ele.

## Consequências

- Um projecto acrescenta o `seiva-kms` (com `vault` se precisar), implementa o
  `RingStore` sobre a sua base de dados e o `Reseal` sobre as suas tabelas de
  segredos, e chama o `RingKeeper::boot` antes de ficar `ready` (ADR 0012).
- Os segredos selados pelo crate ficam presos a `org|propósito|registo`; um
  projecto tem de escolher bem o id do registo (estável durante a vida da linha) e
  mantê-lo igual ao re-selar.
- O build por omissão ganha só o `chacha20poly1305` e o `aes-gcm` da RustCrypto
  (MIT/Apache) e o `zeroize`. Com `vault`, reqwest + rustls (ring). Ambos passam o
  `cargo deny`.
- O `cargo test --workspace` corre os testes do anel, do `local` e do keeper. O
  job de CI `kms-vault` corre os testes do adaptador e os testes live contra um
  contentor de serviço `hashicorp/vault` (transit, rotação da chave do transit e
  o novo embrulho, rotação e retirada da chave de dados, AppRole com novo login).
- Ainda por cobrir: a autenticação Kubernetes e um Vault em HTTPS com CA privada
  estão implementados mas testados só até à configuração (não há cluster nem Vault
  TLS no CI); assinar no guardião (Ed25519 no Vault transit) para chaves que nunca
  devem sair dele; os KMS de nuvem.
- O dxstorage mantém o seu `kms.rs` até uma fatia sua adoptar o crate (depois da
  F15); os seus anéis e blobs `kms1:` lêem-se tal como estão.

## Relacionados

- ADR 0002: portos e duplos de teste em memória (o `MemoryRingStore` segue-o).
- ADR 0012: prontidão (o `boot` a falhar fechado mantém o serviço não `ready`).
- ADR 0036: MFA (as sementes TOTP são os primeiros segredos a entrar num anel).
- ADR 0047: TLS mútuo, o mesmo caminho de "capacidade partilhada passa para o Seiva".

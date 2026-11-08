# ADR 0017 — Hashers de password plugáveis + um fluxo de credenciais de referência

- Estado: Aceite
- Data: 2026-11-06
- Decisores: NgolaCloud
- Segue: ADR 0006

## Contexto

O `seiva-auth` faz hash de passwords com Argon2id e verifica-as, ambos como
**funções livres** fixadas a `Argon2::default()` (`hash_password`, `verify_password`).
A saída é uma string PHC auto-descritiva, mas a verificação corre sempre Argon2id:
um hash bcrypt/PBKDF2/scrypt não pode ser verificado, e não há forma de adicionar um
algoritmo, mudar parâmetros à medida que o hardware melhora, ou migrar um hash
legado — sem editar ou fazer fork do crate.

O modelo do Django é a referência: `PASSWORD_HASHERS` é uma **lista ordenada** de
backends de hasher. O primeiro é usado para passwords novas; qualquer um deles pode
*verificar* um hash existente (para se poderem importar utilizadores de outro
sistema); e quando um hash guardado foi feito por um hasher não-preferido ou com
parâmetros mais fracos, o Django **re-hasheia-o de forma transparente no próximo
login bem-sucedido** (`needs_rehash`). Estende-se adicionando um hasher à lista —
nunca se reescreve o código de autenticação.

O `seiva-auth` também traz um `LoginThrottle` (bloqueio de conta, OWASP API2), mas é
uma primitiva solta ligada a **nada**: nenhum caminho de código compõe "verificar
bloqueio → verificar password → registar sucesso/falha", por isso o bloqueio não
está de facto em serviço em login nenhum.

## Decisão

**Introduzir um trait `PasswordHasher` e um registo ordenado `PasswordHashers`
(o `PASSWORD_HASHERS` do Django), e um fluxo de referência `authenticate_password`
que compõe o throttle, o registo e o rehash-on-login — tudo de forma aditiva,
mantendo as funções livres existentes a funcionar.**

- `trait PasswordHasher` (`id`, `hash`, `identifies`, `verify`, `needs_rehash`),
  `Send + Sync`, para que um registo possa viver atrás de um `Arc`.
- `PasswordHashers`: uma lista ordenada; `hashers[0]` é **preferido** (usado para
  hashes novos). `hash()` usa o preferido; `verify()` encaminha para o primeiro
  hasher que `identifies` a string PHC (para que hashes legados/de outro algoritmo
  verifiquem); `needs_rehash()` é verdadeiro quando o hash guardado não é o algoritmo
  preferido, ou o hasher preferido reporta que os seus parâmetros divergiram do
  default actual.
- Traz `Argon2idHasher` como default/preferido. `PasswordHashers::default()` é só
  Argon2id; `hash_password`/`verify_password` **delegam** nele, por isso todo o
  chamador existente (o exemplo accounts, o CLI) fica inalterado.
- `authenticate_password(throttle, hashers, key, password, stored_phc, now)` — o
  fluxo de credenciais de referência: recusa se a chave está bloqueada
  (→ `LockedOut(retry)`), verifica através do registo, `record_success` + reporta
  `needs_rehash` num acerto, `record_failure` num desacerto (→ `BadCredentials`).
  Um hash guardado malformado é um erro de dados (`Auth`), não um palpite, e não é
  contado contra o throttle. O endpoint de login de uma aplicação chama isto, depois
  re-hasheia e persiste quando `needs_rehash` está activo, e mapeia `LockedOut` para
  `429 Retry-After`.

O transporte e o armazenamento de utilizadores ficam fora de âmbito (ADR 0006,
agnóstico de ORM): esta é a primitiva componível a partir da qual um endpoint de
login é construído, não uma view HTTP de login.

## Alternativas consideradas

1. **Manter o único Argon2id fixo.** O mais simples, e o Argon2id é um bom default.
   Rejeitada: falha o objectivo explícito — não se pode adicionar um algoritmo,
   afinar parâmetros, ou verificar um hash importado/legado sem fazer fork do crate,
   e não há caminho de upgrade-on-login.
2. **Um `enum Algorithm { Argon2id, Bcrypt, … }` em vez de um trait.** Mais simples
   do que um objecto de trait. Rejeitada: um enum fechado não é extensível por crates
   a jusante — adicionar um algoritmo continua a significar editar o `seiva-auth`,
   que é exactamente o que queremos evitar. Um trait mantém o ponto de extensão
   aberto.
3. **Deixar o `LoginThrottle` por ligar e documentar como usá-lo.** Rejeitada: a
   revisão descobriu que o bloqueio não está em serviço precisamente porque nada o
   compõe com a verificação; um fluxo de referência fecha a OWASP API2 a sério e dá
   às aplicações um default correcto em vez de uma receita fácil de errar.

## Consequências

Mais fácil:

- Adicionar um hasher (bcrypt, scrypt, um algoritmo futuro) implementando um trait e
  colocando-o na lista — sem edições ao código de autenticação. Importar
  utilizadores de outro sistema (os seus hashes verificam) e fazer-lhes upgrade
  transparente no login.
- O `LoginThrottle` está de facto em serviço através do `authenticate_password`
  (OWASP API2), com bloqueio, limpeza-em-sucesso e o sinal de rehash num só sítio.

Mais difícil / dívida aceite (2026-11-06):

- Mais uma superfície pública para manter estável (o trait + registo + fluxo).
  Mitigada por manter as funções livres como o caminho simples e o registo como o
  extensível.
- A framework continua a distribuir só Argon2id; os hashers bcrypt/PBKDF2 (e as suas
  dependências) ficam para a aplicação ou uma adição posterior e separada, para que a
  árvore de dependências default não cresça.

Imposição:

- `cargo test -p seiva-auth`: o registo verifica o preferido **e** um segundo hasher
  (de teste), o `needs_rehash` sinaliza um hash legado/mais fraco, e o
  `authenticate_password` está coberto para bloqueio, sucesso-limpa-o-throttle e
  rehash-on-login. O `seiva-security-review` (API2/API3) corre na alteração.

## Veredicto de legibilidade

Legível sem o código à frente: um programador Rust de fora aprende que o Seiva
adopta o modelo de hashers ordenados do Django (preferido-para-novos, verifica-qualquer,
rehash-no-login) via um trait + registo, que os one-liners existentes continuam a
funcionar, e que um `authenticate_password` de referência finalmente põe o throttle
de bloqueio de conta em serviço — com a fronteira deliberada de que o armazenamento
de utilizadores e a view HTTP de login continuam a ser da aplicação.

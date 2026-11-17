# ADR 0030 — A camada de dados é plugável: o DORM é o adaptador por omissão, não um requisito; sqlx e SeaORM são de primeira classe

- Estado: Proposto
- Data: 2026-11-15
- Decisores: NgolaCloud
- Segue: ADR 0002 (ports do núcleo e duplos in-memory), ADR 0004 (o gateway de dados DORM), ADR 0011 (idempotência como port, reutilizando o DORM), ADR 0017 (hashers de palavra-passe plugáveis + um fluxo de referência)

## Contexto

A fronteira já é neutra. O `seiva-ports` não tem nenhuma dependência da camada de
dados e nenhuma assinatura deixa escapar um tipo de ORM: `UnitOfWork`,
`EventPublisher`, `Clock`, `IdGenerator`, `IdempotencyStore`, `AuditSink` e
`RateLimitStore` estão expressos em termos de `Scope`, `Id`, `PortResult` e
primitivos, nunca `Session`, `QueryBuilder` ou SQL. Cada módulo define o seu
próprio port de repositório no seu `domain`, em termos das suas próprias entidades
(`examples/notes/domain` → `NoteRepository`). Já existem duas implementações
independentes destes ports — o gateway DORM (`seiva-data-dorm`) e os duplos
in-memory (`seiva-testing`) — o que é a prova viva de que o contrato não está
soldado ao DORM. As camadas de aplicação e de domínio de todos os módulos em
`examples/` não importam ORM algum; a regra de dependência é imposta pelo grafo de
crates e pelo `seiva lint` (ADR 0003).

Portanto, a arquitectura está certa. A **história de produto não está.** Três
coisas fazem o DORM ler-se como o *único* caminho realista em vez do caminho *por
omissão*:

- **Só há um adaptador de referência.** O `seiva-data-dorm` é o único crate que
  implementa os ports de ponta a ponta. Quem queira sqlx ou SeaORM encontra um
  buraco e uma promessa, não um exemplo trabalhado.
- **A CLI cola-se ao DORM.** O `seiva db init/dev/migrate/pull/studio` delega no
  binário `dorm` (`seiva-cli`), e o `seiva module new --data external` responde
  *"not implemented yet"*. Um utilizador sem DORM não obtém scaffold nem caminho de
  migração a partir das ferramentas do próprio framework.
- **O README enquadra a alternativa como um vazio.** Diz para *desligar o DORM e
  trazer as tuas próprias ligações* — um vácuo de faz-tu-mesmo, não um adaptador
  suportado.

Isto importa porque é a maior **barreira de adopção** do framework. Pedir a um
programador Rust que aceite o Seiva **e** um ORM proprietário de uma só vez duplica
o risco percebido, ao lado de um sqlx ou SeaORM que já corre em produção. A
comunidade Rust pega nas stacks em que já confia; um framework full-stack que
*exige* o seu próprio ORM auto-selecciona uma audiência minúscula. Os concorrentes
confirmam a lacuna: o Loco casa-se com o SeaORM, o cot com o seu próprio — nenhum te
deixa trazer a camada de dados que já tens.

As forças:

- **O DORM continua o default, e tem de continuar a valer a pena ser o default.** As
  baterias — migrações, o outbox transaccional (ADR 0011), o admin studio, o schema
  por módulo — são o que separa o Seiva de "axum + sqlx montado à mão". Um projecto
  que arranca do zero deve obter o caminho mais curto, e esse caminho é o DORM.
- **O framework não pode impor um ORM.** A neutralidade que os ports já têm tem de
  se tornar uma *promessa de produto*, não um acidente de arquitectura a um refactor
  de apodrecer.
- **O núcleo mantém-se livre de DORM e livre de ORM.** Tal como o rate limit
  (ADR 0015) e o hasher de palavra-passe (ADR 0017), o mecanismo vive no framework
  mas o backend concreto é escolhido na composition root, atrás de uma feature de
  cargo, não ligando nada quando não é usado.
- **A neutralidade tem de ser imposta, não confiada.** Se `application`/`domain`
  puder nomear um crate de ORM e a CI continuar verde, a fronteira erode-se à
  primeira vez que alguém tem pressa.

## Decisão

**A camada de dados do Seiva é um conjunto de ports (`seiva-ports`); o DORM é o
adaptador de referência por omissão, e sqlx e SeaORM são adaptadores de referência
de primeira classe mantidos ao seu lado. Um projecto escolhe o seu adaptador na
composition root; o núcleo, a aplicação e o domínio nunca nomeiam um ORM.**

**O contrato.** O `seiva-ports` é toda a superfície da camada de dados: os ports de
serviço (`UnitOfWork`, `EventPublisher`, `Clock`, `IdGenerator`, `IdempotencyStore`,
`AuditSink`, `RateLimitStore`) mais o port de repositório por módulo que cada módulo
declara no seu `domain` em termos das suas próprias entidades. Nenhum tipo de ORM
atravessa esta fronteira — um invariante verificável que já se mantém hoje.

**Adaptadores de referência.** Cada um é um crate `seiva-data-<x>` atrás da sua
própria feature de cargo, nenhum nos `members` do workspace por omissão, nenhum
ligado por um crate do núcleo:

- `seiva-data-dorm` (**por omissão**, feature `dorm`): migrações, o outbox
  transaccional, studio, admin — a experiência batteries-included. Inalterado por
  esta ADR.
- `seiva-data-sqlx` (feature `sqlx`): os ports sobre `sqlx::Transaction`; o
  utilizador traz o seu próprio schema e migra com o `sqlx-cli`. Esta é a prova de
  que o Seiva é agnóstico *até de um ORM* — corre sobre uma camada de query nua.
- `seiva-data-seaorm` (feature `seaorm`): os ports sobre o `db.transaction()` do
  SeaORM; o utilizador migra com o `sea-orm-cli`. Esta é a resposta directa ao Loco
  — o mesmo ORM, sob um framework que não to impõe.

**O contrato de transacção é "ambiente".** `UnitOfWork::transaction(scope, closure)`
é baseado em escopo, não em handle: o closure corre dentro da transacção, commit no
`Ok`, rollback no `Err`, e nenhum handle de transacção é passado ao corpo. Cada
adaptador estabelece a transacção corrente no seu próprio task-local para que os
repositórios do módulo a encontrem — DORM via `db::atomic`, sqlx via `begin`/`commit`,
SeaORM via `transaction()`. Este padrão de "transacção ambiente" **é** o contrato que
todo o adaptador cumpre; está documentado como tal, não reinventado por backend.

**Omissão e experiência de programador.** O DORM continua o default do scaffold
(`seiva module new --data dorm`); `--data sqlx | seaorm | external` selecciona outro
adaptador. As ferramentas específicas do DORM (`seiva db migrate/pull/studio`)
continuam a delegar no DORM quando o adaptador *é* o DORM; para sqlx/SeaORM a CLI
aponta para a ferramenta nativa (`sqlx migrate`, `sea-orm-cli`) em vez de fingir. A
mecânica da CLI é uma fatia à parte; esta ADR fixa a **política**, não a canalização.

**A neutralidade torna-se uma fitness function.** O `seiva lint` ganha uma
verificação que falha se um crate `application` ou `domain` nomear um crate de
adaptador de dados (`seiva-data-*`) ou um tipo de ORM. A fronteira unidireccional e
livre de ORM deixa de ser uma convenção e torna-se um gate de CI; a CI constrói os
três adaptadores de referência contra os ports.

## Alternativas consideradas

- **Manter o DORM como o único caminho realista (status quo).** Rejeitado como sendo
  o cerne da questão: é a maior barreira de adopção do framework, afasta toda a gente
  que já tem sqlx/SeaORM em produção, e desperdiça o facto de o contrato *já* ser
  neutro. Uma fronteira neutra que ninguém vê não vale nada.
- **Abstrair um "ORM Seiva" universal que embrulhe qualquer backend** (um query
  builder por cima). Rejeitado: reimplementaria mal o sqlx/SeaORM/Diesel, prenderia
  cada utilizador a uma linguagem de query de menor-denominador-comum, e contradiz
  "não impor um ORM". O Seiva abstrai os **ports de serviço** (transacção,
  idempotência, outbox, auditoria), não a linguagem de query — cada adaptador fala a
  sua.
- **Tornar o sqlx ou o SeaORM o default em vez do DORM.** Rejeitado: o DORM é o que
  entrega a experiência batteries-included (migrações, admin, studio, outbox tudo
  ligado) que distingue o Seiva do axum montado à mão, e é o produto da família. A
  resposta é *DORM por omissão, aberto a todos*, não um default mais fraco.
- **Deixar o utilizador escrever o seu próprio adaptador sem referência (o status quo
  do "desliga-o e traz o teu").** Rejeitado: uma promessa sem um adaptador trabalhado
  não converte — a comunidade tem de *ver* sqlx e SeaORM a compilar contra os ports
  para acreditar. O precedente é a ADR 0017: hashers de palavra-passe plugáveis
  entregues *com* um fluxo de credenciais de referência, não como um trait nu.

## Consequências

- **O posicionamento inverte-se.** O Seiva torna-se "o framework full-stack Rust que
  não te casa com um ORM" — um diferenciador que nem o Loco (SeaORM) nem o cot (o
  próprio) podem reclamar. A barreira de adopção n.º 2 (um ORM proprietário) torna-se
  a funcionalidade de cabeçalho.
- **Há um caminho de migração para dentro.** Uma equipa já em sqlx ou SeaORM adopta o
  Seiva sem reescrever a persistência: implementa os ports, mantém o schema e as
  queries existentes. O custo de experimentar o framework desce de "aprender também
  um ORM novo" para "ligar um adaptador".
- **O DORM continua a ganhar o default.** É a experiência mais rica, não a única; o
  programador que começa do zero continua a obter o caminho mais curto, e o ORM da
  família é o que o quickstart mostra.
- **Dívida que aceitamos (2026-11-15):** três adaptadores de referência para manter —
  a matriz de CI cresce, e cada port novo tem de aterrar nos três. Mitigado por o
  conjunto de ports ser pequeno e estável (ADR 0002) e por os adaptadores sqlx/seaorm
  serem exemplos de referência magros, não produtos; se um port não puder ser cumprido
  por uma camada de query simples, isso é um sinal de que o port está errado, apanhado
  cedo pelo adaptador sqlx.
- **Imposição, não confiança:** a neutralidade é verificada pelo `seiva lint` e a
  fronteira é um gate de CI, pelo que não pode apodrecer em silêncio.
- **Fora desta ADR:** a CLI de migração para adaptadores não-DORM (`seiva db *`,
  `module new --data external`) é uma fatia à parte; até aterrar, os utilizadores de
  sqlx/SeaORM correm a sua própria ferramenta de migração fora do `seiva db`. Esta ADR
  assume a política que torna essa fatia digna de ser feita.

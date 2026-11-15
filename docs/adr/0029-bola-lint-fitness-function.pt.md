# ADR 0029 — Uma fitness function de BOLA no `seiva lint`: DTOs de resposta com âmbito de inquilino têm de ser devolvidos com âmbito, para que a autorização ao nível do objecto deixe de ser um sistema de honra

- Estado: Proposto
- Data: 2026-11-15
- Decisores: NgolaCloud
- Segue: ADR 0005 (autorização ao nível do objecto / BOLA), ADR 0019 (autorização fail-closed), ADR 0003 (camadas de módulo e as fitness functions do `seiva lint`)

## Contexto

O ADR 0005 deu ao Seiva um primitivo forte de autorização ao nível do objecto:
`Scoped<T>` (`crates/seiva-http/src/guard.rs`) não tem `into_inner`, nem campo
público, nem `Deref`; a única saída é `owned_by(&Principal)`, que devolve `403`
num desencontro de inquilino, e é `#[must_use]`. Onde um módulo o usa, esquecer a
verificação é **erro de compilação** — BOLA fechado por tipo.

Mas a adopção é **opt-in e por módulo**. O próprio ADR 0005 regista a lacuna:
*«Módulos que devolvem DTOs crus ficam no sistema de honra.»* Um módulo novo que
carregue um registo por id e devolva o seu DTO **sem** `owned_by`/âmbito de
inquilino serve o objecto de um inquilino a outro com um `200`, passando por cima de
toda a guarda a nível de função (`#[requires("perm")]` verifica o *verbo*, não o
*objecto*). O framework entrega o primitivo de segurança mas **não força o seu uso e
não detecta a sua ausência** — a forma exacta do risco de API número um (BOLA,
OWASP API1).

As forças:

- O Seiva já trata regras de arquitectura como **fitness functions executáveis**: o
  `seiva lint` impõe a regra de dependência de camadas do ADR 0003 contra o grafo de
  crates. A adopção de BOLA deve ser o mesmo tipo de gate — uma regra que o CI
  verifica, não uma convenção que o revisor tem de lembrar.
- Uma análise completa de fluxo de dados («chega um campo `tenant` a uma resposta
  sem passar por `Scoped`?») é demasiado ambiciosa e demasiado frágil para correr
  estaticamente sobre Rust real, e um lint com falsos positivos treina as pessoas a
  ignorá-lo. A regra tem de ser **barata, local e de sinal alto**, construída sobre
  informação que o framework já tem em tempo de compilação: o descritor da fachada e
  o `WireSchema` de cada DTO no fio (ADR 0007).
- Tem de permanecer **fail-closed** (ADR 0019): o *padrão* para um DTO de resposta
  com âmbito de inquilino devolvido sem anotação de âmbito é **erro**, não passar. A
  opção de saída tem de ser **explícita e auditável**, nunca silenciosa.
- Não deve quebrar módulos existentes gratuitamente: os módulos de referência já
  aplicam âmbito correctamente, pelo que a regra deve passar neles e só disparar na
  forma do sistema de honra.

## Decisão

Adicionar uma fitness function `bola` ao `seiva lint` (a mesma ferramenta que impõe
as camadas do ADR 0003), corrida no CI como gate duro.

**O que inspecciona.** Para cada método de fachada, o lint lê o descritor de tempo
de compilação já produzido por `#[seiva::facade]` e o `WireSchema` de cada DTO de
resposta (ambos existem hoje — sem nova reflexão):

1. **Classifica o DTO de resposta como «com âmbito de inquilino»** quando o seu
   `WireSchema` (incluindo structs derivadas aninhadas) contém um campo cujo nome
   corresponde a um conjunto configurável — por omissão `tenant`, `tenant_id`,
   `owner`, `owner_id`. São estes os DTOs cuja exposição entre inquilinos é uma fuga
   BOLA.
2. **Exige uma decisão de âmbito no método que devolve.** Um método que devolve um
   DTO com âmbito de inquilino tem de satisfazer uma de:
   - devolvê-lo como `Scoped<T>` / uma colecção com âmbito (o caminho imposto por
     tipo do ADR 0005), **ou**
   - levar um atributo explícito `#[scoped]` que afirma que a camada de aplicação
     dá âmbito à consulta por `&Ctx` (o padrão que os módulos de referência usam,
     onde o âmbito é derivado do lado do servidor a partir do principal e empurrado
     para o gateway DORM), **ou**
   - levar uma opção de saída explícita `#[cross_tenant(reason = "...")]` — um
     marcador **auditável** para uma leitura de administração/relatório
     legitimamente entre inquilinos.
3. **O padrão é erro.** Um DTO de resposta com âmbito de inquilino devolvido por um
   método sem nenhuma das três é um **erro de lint** que faz falhar o CI.
   Fail-closed: o caso não marcado não passa.

**Relatório.** O erro nomeia a fachada, o método e o campo ofensor do DTO, e declara
as três formas de o resolver — a mesma fasquia ergonómica do lint de camadas, para
que a correcção seja óbvia a partir da mensagem (o estilo de diagnóstico do
ADR 0003).

**A saída é auditável, não invisível.** O `#[cross_tenant(reason = "...")]` é
greppável e revisível; uma revisão de segurança pode enumerar toda leitura
deliberadamente entre inquilinos na base de código só a partir das anotações.

## Consequências

- A maior lacuna estrutural de segurança — a adopção de BOLA deixada à disciplina —
  torna-se um gate de CI. Um módulo novo que devolva um DTO com âmbito de inquilino
  sem lhe dar âmbito **faz falhar o build**, pelo que o sistema de honra é
  substituído por uma fitness function, coerente com a forma como o ADR 0003 impõe
  as camadas.
- O custo para os autores de módulos é uma anotação explícita na rara leitura entre
  inquilinos e nada na leitura com âmbito comum (que já devolve `Scoped<T>` ou deriva
  o âmbito de `&Ctx`). Os módulos de referência passam inalterados.
- A revisão de segurança ganha um inventário: cada `#[cross_tenant]` é uma excepção
  auditável e justificada, em vez de um retorno de DTO cru invisível que ninguém
  sinalizou.
- É uma **heurística, não uma prova**: a classificação por nome de campo pode falhar
  um DTO com âmbito de inquilino que nomeie o seu discriminador de forma pouco
  convencional, ou sinalizar um campo chamado `owner` que não é chave de inquilino. O
  conjunto configurável de nomes e a válvula de escape `#[cross_tenant]` mantêm ambos
  os modos de erro baratos de resolver, e a regra é segurança estritamente aditiva
  sobre as zero verificações de hoje.
- Não substitui o `Scoped<T>` (ADR 0005) — **conduz a sua adopção**: o lint empurra
  os autores para o caminho imposto por tipo, onde esquecer a verificação já é erro
  de compilação.

## Alternativas consideradas

- **Análise estática completa de fluxo de dados** (seguir um valor de inquilino do
  carregamento à resposta). Rejeitada: demasiado cara e demasiado frágil sobre Rust
  real; os falsos positivos levariam a desligar o lint. A heurística
  descritor+`WireSchema` é barata, local e corre sobre informação já presente.
- **Uma asserção em runtime** (panic/`403` se um DTO com âmbito de inquilino sair sem
  âmbito). Rejeitada: não é fail-closed em tempo de compilação, só dispara no
  caminho de código que executa (pelo que um caminho não testado ainda vaza), e move
  uma garantia de tempo de build para produção.
- **Um lint Clippy próprio.** Rejeitado: o Clippy vê a AST, não o descritor da
  fachada nem o `WireSchema`, pelo que não consegue raciocinar sobre *DTOs de
  resposta de métodos de fachada*; o `seiva lint` já detém esse modelo (ADR 0003).
- **Deixar à revisão de código (statu quo).** Rejeitado como a razão inteira do ADR:
  a revisão é exactamente o sistema de honra que o ADR 0005 sinalizou, e BOLA é o
  risco de API número um — merece um gate, não a memória de um revisor.

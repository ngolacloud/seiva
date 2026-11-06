# ADR 0005 — Autorização ao nível do objecto (BOLA / API1): fail-closed por tipo

- Estado: Aceite
- Data: 2026-11-04
- Decisores: NgolaCloud
- Segue: ADR 0003

## Contexto

O Seiva já impõe autorização **ao nível da função** (OWASP API5): o middleware
`permission_guard` lê a permissão que uma rota exige a partir do
`MethodDescriptor` da fachada e recusa um chamador que não a tenha. Isso responde
a "pode este chamador sequer chamar esta operação".

Não responde à autorização **ao nível do objecto** (OWASP API1, Broken Object
Level Authorization — BOLA): "pode este chamador tocar *neste recurso
específico*". A BOLA é a vulnerabilidade de API mais comum e mais danosa
precisamente porque as verificações ao nível da função passam — o chamador é um
utilizador legítimo com a permissão certa — enquanto o objecto pertence a outro
inquilino. Um middleware de rota não lhe consegue responder: o middleware corre
*antes* de o handler carregar o recurso, por isso ainda não sabe que inquilino é
dono dele.

O framework trazia `Principal::owns` / `Principal::require_owns` como peça de
construção para isto. Mas são **consultivos**: nada força um handler a chamá-los.
Um handler que carrega um registo e o devolve, esquecendo-se de `require_owns`,
serve os dados de um inquilino a outro — silenciosamente, com um `200`, passando
por todos os portões existentes. A lacuna é uma questão de disciplina do
programador, que é exactamente o que um framework de referência deve remover, não
em que confiar.

As forças:

- A verificação precisa do inquilino do recurso, por isso só pode acontecer
  *depois* do carregamento — na costura aplicação/handler, não no limite.
- "Esquecer a verificação" não pode ser um erro silencioso e fail-open.
- A regra de camadas (ADR 0003) proíbe `interface → domain` e
  `application → delonix-seiva`/transporte, por isso onde quer que a primitiva
  viva restringe quem a pode construir.

## Decisão

**Tornar o caminho não verificado irrepresentável com um invólucro etiquetado por
inquilino, `Scoped<T>`, cujo valor interno só é alcançável passando a
verificação de propriedade.** `require_owns`/`owns` mantêm-se como a válvula de
escape explícita.

1. `Scoped<T>` leva o `tenant` de um recurso ao lado do seu valor. É construído
   onde o recurso é carregado, a partir da própria coluna de inquilino do
   registo: `Scoped::new(record.tenant, dto)`.

2. A **única** forma de ler o valor é
   `Scoped::<T>::owned_by(self, &Principal) -> Result<T, Problem>`, que devolve o
   `T` se e só se o principal for dono do inquilino e um `403` `Problem` caso
   contrário. Não há `into_inner`, não há campo público, não há `Deref`. Um
   handler que se esqueça de verificar não consegue, por isso, alcançar o recurso
   para o devolver — a omissão é um **erro de compilação**, não uma fuga em
   runtime. `Scoped<T>` é `#[must_use]`, por isso até deixá-lo cair no chão avisa.

3. `Principal::require_owns`/`owns` mantêm-se para os casos em que um invólucro
   não encaixa — p. ex. uma lista já filtrada por `WHERE tenant = $1` em SQL,
   onde a fronteira do inquilino foi imposta na query e não há valor por item para
   desembrulhar. Usá-los é então uma asserção deliberada e visível, não uma
   omissão silenciosa.

## Alternativas consideradas

- **Manter `require_owns` apenas consultivo (status quo).** Rejeitada: falha
  aberto. Uma chamada esquecida é invisível — sem erro de tipo, sem falha de
  teste, sem sinal de CI — e a BOLA é demasiado cara para se deixar à disciplina.

- **Um middleware de rota ao nível do objecto, como o `permission_guard`.**
  Rejeitada por impossível, não meramente indesejável: a identidade do objecto é
  desconhecida até o handler ter carregado o recurso, por isso um middleware corre
  necessariamente cedo de mais para conhecer o inquilino. As verificações ao nível
  da função e ao nível do objecto vivem, por natureza, em pontos diferentes da
  vida do pedido.

- **Declarar a política do objecto no descriptor e impô-la no controller gerado,
  lendo um inquilino do DTO de resposta.** Rejeitada *por agora*: força cada DTO
  pertencente a um inquilino a expor o seu inquilino através de um trait
  `HasTenant`, acopla a camada de transporte a um conceito de inquilinato de
  domínio, e ainda assim não cobre recursos que nunca atravessam o fio. Uma
  alteração de contrato mais pesada para cobertura mais estreita do que o
  invólucro. Deixada como possível camada futura sobre o `Scoped`.

- **`Scoped<T>` — a capacidade ao nível do tipo (escolhida).** A menor alteração
  que vira o modo de falha de aberto para fechado, não precisa de alterar
  descriptor nem macro, lê-se claramente a um programador de fora, e compõe com o
  `Principal` existente.

## Consequências

- **Mais fácil:** um revisor que vê um `Scoped<T>` chegar a uma resposta sabe que
  passou a verificação de propriedade — o tipo di-lo. A lacuna BOLA torna-se um
  erro de compilação que o programador encontra enquanto escreve o handler, não
  uma fuga descoberta em produção.

- **Mais difícil / dívida aceite (2026-11-04):** a adopção é por módulo e
  opcional — um módulo só ganha a garantia quando o seu caso de uso/handler
  enfiar `Scoped<T>`. Módulos que devolvem DTOs crus ficam no sistema de honra até
  o adoptarem. Rastreado em `docs/progress.md`.

- **Seguimento de camadas (2026-11-04):** `Scoped<T>` e `Principal` vivem hoje em
  `seiva-http`, por isso o invólucro é construído na **costura
  interface/handler** (a partir do inquilino do DTO), não dentro de `application`
  — um caso de uso que devolvesse `Scoped<T>` directamente puxaria transporte
  para `application`, o que a ADR 0003 proíbe. Mover a primitiva de propriedade
  para uma crate sem transporte (`seiva-auth` ou um kernel) para que `application`
  possa devolver `Scoped<T>` ela própria é a refactorização que o primeiro módulo
  multi-inquilino real vai forçar; adiada até lá em vez de adivinhada agora.

- **Imposição:** o próprio tipo é a fitness function — sem `into_inner`, sem
  campo público, sem `Deref`, `#[must_use]` no invólucro. Uma futura verificação
  `seiva lint` que sinalize um DTO de fachada com um campo `tenant` cujo caminho
  do handler chega a uma resposta sem passar por `Scoped`/`require_owns` está
  listada como trabalho de seguimento, não entregue aqui.

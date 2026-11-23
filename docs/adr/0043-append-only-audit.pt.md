# ADR 0043 — Um log de auditoria append-only, encadeado por hash: uma implementação `AuditSink` à prova de adulteração

- Status: Proposto
- Data: 2026-11-23
- Decisores: NgolaCloud
- Segue: ADR 0002 (core ports e doubles in-memory), ADR 0013 (auditoria persistente de mutações administrativas), ADR 0019 (autorização fail-closed)

## Contexto

O Seiva já tem o **port `AuditSink`** e o middleware do ADR 0013 que regista por
ele cada mutação administrativa (actor, tenant, operação, resultado, id de
correlação, detalhes redigidos), com um sink apoiado em DORM a fazer append a
`seiva_audit`. Isso responde a *«foi registado?»*. Ainda não responde a *«pode
confiar-se no registo — alguém editou ou apagou uma linha após o facto?»*. Para uma
plataforma soberana, um operador (ou um atacante que alcance a base de dados)
poderia fazer `UPDATE` ou `DELETE` a uma linha de auditoria e apagar os seus
rastos, e nada o detectaria. Uma implantação regulada ou sensível à segurança
precisa de um log de auditoria **append-only e à prova de adulteração**: só
escritas, e qualquer edição após o facto é *visível*.

Este ADR não substitui o ADR 0013; **estende** o seu `AuditSink` com uma
implementação cujo armazenamento é imutável e cujos registos são encadeados para
que a adulteração seja detectável, mais retenção e exportação.

As forças:

- **Append-only, imposto, não meramente pretendido.** «Não editar linhas de
  auditoria» como convenção não vale nada contra um operador comprometido. A
  implementação tem de tornar `UPDATE`/`DELETE` *impossível ou detectável*: um sink
  só-de-escrita, grants de base de dados que só permitem `INSERT`, e
  mutate/delete revogado na tabela de auditoria — o próprio armazenamento recusa a
  edição.
- **A prova de adulteração é uma propriedade criptográfica, não um nível de log.**
  O armazenamento append-only trava edições casuais, mas um actor privilegiado que
  consiga reescrever a tabela ainda precisa de ser *apanhado*. Uma **hash chain** —
  cada registo guarda o hash do registo anterior mais o seu próprio conteúdo —
  significa que alterar ou remover qualquer linha parte a cadeia a partir desse
  ponto, e uma passagem de verificação encontra exactamente onde.
- **Uma auditoria que pode ser silenciosamente saltada não é uma auditoria.** Para
  as operações que legalmente *têm* de ser auditadas, uma escrita de auditoria
  falhada tem de **falhar a operação** (fail-closed, ADR 0019), não ser
  registada-e-engolida. Esta é uma postura mais estrita do que o middleware
  best-effort do ADR 0013, aplicada ao subconjunto crítico.
- **Minimização de PII.** Os registos de auditoria descrevem *quem fez o quê a
  quê*, não o payload secreto (o ADR 0013 já redige). O estado before/after é
  guardado como **hash** por omissão, não os valores crus, para que o log prove que
  uma alteração ocorreu sem ele próprio se tornar uma segunda cópia de dados
  sensíveis.
- **A retenção é política; o mecanismo é particionamento.** Quanto tempo manter, e
  o que a lei exige, são política do produto/operador. O framework fornece
  particionamento por tempo, imposição de retenção, e exportação para que essa
  política seja barata de aplicar.

## Decisão

Adicionar uma **implementação `AuditSink` append-only, encadeada por hash** — um
adapter adicional atrás do port existente, não um port novo — mais o seu
ferramental de verificação e exportação.

- **O registo encadeado** — estende `AuditEntry` (ADR 0013: actor, action,
  resource, scope, timestamp, request-id, detalhes redigidos) com um `prev_hash` e
  um `entry_hash = H(prev_hash ‖ canonical(entry))`. O estado `before`/`after` é
  guardado como hash por omissão (o diff cru só quando o produto opta por isso e
  não é sensível). Os registos formam uma cadeia por-tenant (ou global).
- **Armazenamento só-de-escrita.** O adapter DORM/sqlx persiste numa tabela de
  auditoria cujos grants permitem **só `INSERT`** — `UPDATE` e `DELETE` são
  revogados para o papel da aplicação; combinado com particionamento por tempo,
  partições antigas podem ser seladas. O sink também **não** expõe método de update
  ou delete ao nível do port.
- **Uma passagem de verificação** — `verify_chain(range)` recomputa cada
  `entry_hash` a partir de `prev_hash` + conteúdo e reporta a primeira quebra, para
  que a adulteração (uma linha editada ou removida) seja localizada, não apenas
  suspeitada. Pode correr em calendário (`seiva-jobs`, ADR 0034) e levantar um
  alerta numa quebra.
- **Fail-closed para o subconjunto crítico.** Um wrapper `CriticalAudit` escreve o
  registo **duravelmente antes** de a operação auditada fazer commit e torna uma
  escrita falhada em **falha da operação**. O middleware de metadados do ADR 0013
  fica best-effort para a corrente larga de mutações; este modo mais estrito é
  opt-in para as acções legalmente exigidas.
- **Retenção e exportação.** O particionamento por tempo (mensal, digamos) torna a
  retenção num drop de partição e mantém o caminho quente pequeno; um
  `export(range)` produz um arquivo assinado para armazenamento frio ou um SIEM
  externo, carregando a cadeia para que a exportação seja independentemente
  verificável.

A política — *quais* acções exigem auditoria crítica fail-closed, e a janela de
retenção legal — fica com o produto (o ADR 0013 já cataloga as mutações admin). O
mecanismo — imutabilidade, encadeamento, verificação, exportação — é do framework.

## Alternativas consideradas

- **Manter só a tabela best-effort do ADR 0013.** Rejeitado para implantações
  reguladas: uma tabela mutável com um escritor best-effort nada prova contra um
  operador que a edite; append-only + hash-chain é o que torna o log *digno de
  confiança*.
- **Enviar a auditoria para um SIEM externo e chamar-lhe imutável.** Rejeitado como
  base: isso é um *alvo de exportação* válido, mas a escrita de primeira-parte tem
  de ser à prova de adulteração por si para que o pipeline para o SIEM seja ele
  próprio auditável. A exportação complementa, não substitui.
- **Assinar cada registo individualmente em vez de encadear.** Rejeitado:
  assinaturas por-registo provam autenticidade mas não *completude* — uma linha
  apagada não deixa lacuna. Uma hash chain detecta remoção e reordenação, que é a
  ameaça.
- **Guardar sempre os valores before/after completos.** Rejeitado: isso re-copia
  dados sensíveis para o log de auditoria (a coisa que o ADR 0013 proíbe).
  Hash-por-omissão, cru-só-com-opt-in-explícito mantém o log mínimo.

## Consequências

- **Performance.** As escritas são **appends sequenciais**; o particionamento por
  tempo mantém a partição viva pequena e os índices apertados; um caminho de
  escrita assíncrono opcional (via `seiva-jobs`) absorve rajadas **sem perder
  ordem**, já que a cadeia impõe a ordenação independentemente do tempo de escrita.
- **Segurança.** O log é **imutável** (grants só-INSERT, sem update/delete no
  port); a **hash chain** torna qualquer edição, remoção ou reordenação detectável;
  o before/after é hashed para minimizar PII; o subconjunto crítico é
  **fail-closed** (ADR 0019) — uma escrita de auditoria falhada aborta a operação
  auditada em vez de prosseguir sem registo.
- **Resiliência.** A cadeia é **independentemente verificável** a qualquer momento,
  pelo que a confiança não depende do bom comportamento do escritor; o caminho
  crítico escreve duravelmente **antes** de a operação fazer commit, pelo que um
  crash não pode deixar uma operação já commitada por auditar; a exportação carrega
  a cadeia para verificação independente fora do sítio.
- O requisito de auditoria regulada do delonix-meet é cumprido trocando para este
  sink atrás do port que já usa — sem alterações de call-site — e toda a plataforma
  que tenha de provar que a sua auditoria não foi adulterada ganha-o de graça.

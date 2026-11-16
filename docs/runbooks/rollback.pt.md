# Runbook: rollback

**Usar quando:** uma versão está má — erros novos, readiness partida, uma regressão
— e tem de ser revertida **já**. Reverta primeiro, diagnostique depois.

## Decidir: reverter ou avançar?

Reverta (**roll back**) quando a versão anterior era reconhecidamente boa e a
correcção não é imediata. Avance (**roll forward**) só quando a correcção é
trivial, já está compilada, e é mais rápida de enviar do que uma reversão. Na
dúvida, reverta — é a jogada reversível.

## Reverter com Helm

O Helm guarda um histórico de revisões; um comando reverte toda a release (imagem,
configuração, tudo) para a revisão anterior.

```console
$ helm history seiva                      # encontrar a última REVISION boa
$ helm rollback seiva <revisão> --wait    # omitir <revisão> volta exactamente uma
$ kubectl rollout status deploy/seiva-seiva-server
```

As mesmas garantias de `RollingUpdate` valem ao contrário: os pods da versão boa
têm de passar as sondas antes de os maus serem removidos, por isso o rollback nunca
desce abaixo do número de réplicas. O `PodDisruptionBudget` (`minAvailable: 1`)
mantém pelo menos uma réplica a servir durante todo o processo.

## Reverter com kubectl (sem Helm)

Se a mudança foi aplicada com `kubectl` em vez de Helm:

```console
$ kubectl rollout history deploy/seiva-seiva-server
$ kubectl rollout undo deploy/seiva-seiva-server          # revisão anterior
$ kubectl rollout undo deploy/seiva-seiva-server --to-revision=<n>
$ kubectl rollout status deploy/seiva-seiva-server
```

## Gracioso por construção

Um rollback é seguro de correr com tráfego vivo porque o encerramento é gracioso:

- Com `SIGTERM` o pod vira a readiness para `Draining` (`/health/ready` → `503`),
  por isso o balanceador deixa de lhe enviar pedidos novos, enquanto `/health/live`
  se mantém `200` para o Kubernetes não o matar a meio da drenagem.
- Os pedidos em curso terminam dentro de `terminationGracePeriodSeconds` (por
  omissão `30`).
- Os workers de fundo observam o mesmo sinal de encerramento e param após a passagem
  actual, por isso nenhum trabalho é interrompido a meio.

## Se o próprio rollback não arrancar

Trate-o como uma falha de arranque: siga o [runbook de arranque](startup.md). A
causa mais comum é um **desajuste de configuração/esquema** — a versão má migrou
uma base de dados ou mudou a forma de um ConfigMap que o binário antigo não
consegue ler. Nesse caso um rollback só de código/imagem não chega; tem também de
reverter a mudança incompatível de configuração ou dados. É por isto que as
**migrações retrocompatíveis** importam: uma versão que não se consegue reverter
não está verdadeiramente implantada, só apostada.

## Depois de o serviço estabilizar

- Capture a tag da imagem má e o sintoma de falha antes de alguém reimplantar.
- Guarde os logs do pod da versão má: `kubectl logs <pod-mau> --previous`.
- Abra o postmortem; a correcção passa pelo rollout normal, não por um hotfix nos
  pods em execução.

## Terminado quando

- `helm history seiva` (ou `kubectl rollout history`) mostra a revisão boa como
  actual.
- `kubectl rollout status` reporta completo.
- `/health/ready` está `200` em todos os pods e a taxa de erro voltou à linha de
  base.

## Relacionado

- [Arranque e rollout](startup.md) · [Degradação de dependência](dependency-degradation.md)

---

*Parte da [documentação do Seiva](../index.md).*

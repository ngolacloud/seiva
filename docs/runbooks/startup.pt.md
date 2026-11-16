# Runbook: arranque e rollout

**Usar quando:** está a implantar uma nova versão, ou um pod está preso e não fica
`Ready`.

## Rollout normal

O chart usa um `RollingUpdate` com `maxUnavailable: 0` e `maxSurge: 1`: um pod novo
tem de passar nas sondas antes de um antigo ser removido, por isso um rollout
saudável nunca desce abaixo do número de réplicas em serviço.

```console
$ helm upgrade --install seiva deploy/seiva-server \
    --set image.tag=<nova-versão> --wait
$ kubectl rollout status deploy/seiva-seiva-server
```

O ciclo de vida de um pod, como as sondas o vêem:

1. **`startupProbe`** sonda `/health/live` até o processo aceitar ligações.
   Enquanto não passar, liveness e readiness ainda não são consultadas, por isso um
   arranque lento nunca é confundido com um crash loop.
2. **`readinessProbe`** (`/health/ready`) fica `503` enquanto o processo está
   `Starting` e enquanto a verificação de dependência crítica falha, por isso não
   se encaminha tráfego até o serviço o poder servir de facto. A raiz de composição
   chama `Health::set_ready()` quando a cablagem termina.
3. **`livenessProbe`** (`/health/live`) está `200` desde o momento em que o
   processo está de pé.

## Um pod não fica Ready

Confirme o que as sondas reportam:

```console
$ kubectl get pods -l app.kubernetes.io/name=seiva-server
$ kubectl describe pod <pod>          # ver os Events e falhas de sonda
$ kubectl port-forward <pod> 8080     # depois, noutro terminal:
$ curl -s -o /dev/null -w '%{http_code}\n' localhost:8080/health/live
$ curl -s localhost:8080/health/ready  # o corpo diz PORQUÊ está unready
```

Decida a partir das duas sondas:

| `/health/live` | `/health/ready` | Significado | Acção |
|----------------|-----------------|-------------|-------|
| `200` | `503` `"state":"starting"` | Ainda a cablar | Esperar; se nunca virar, a raiz de composição nunca chamou `set_ready()` — ver os logs de arranque por um panic/pendura antes de a readiness ser marcada |
| `200` | `503` `"state":"dependency-unready"` | De pé, mas uma dependência crítica está inacessível | É **degradação de dependência** — ver [esse runbook](dependency-degradation.md) |
| ligação recusada | — | Processo não está à escuta | Confirmar que `SEIVA_HTTP_ADDR` é `0.0.0.0:<porta>` (o chart define-o); um bind em `127.0.0.1` é inacessível à sonda |
| crash loop | — | Arranque falha | `kubectl logs <pod> --previous`; um erro de configuração sai não-zero com uma linha JSON a nomear a variável má |

## Falhas de arranque comuns

- **Configuração inválida.** O `seiva-server` valida o ambiente no arranque e sai
  não-zero, registando cada problema. `kubectl logs <pod> --previous` mostra a
  variável em falta (p. ex. um `SEIVA_OIDC_ISSUER` em falta quando o trio OIDC está
  parcialmente definido). Corrija o ConfigMap/Secret e re-role.
- **Guarda de segurança do `AppEnv`.** Ligar um atalho de desenvolvimento (p. ex.
  `SEIVA_EXPOSE_ERROR_DETAILS`) com `APP_ENV=production` é um **erro de arranque**,
  por desenho. Remova o atalho ou não corra essa imagem em produção.
- **Escrita com readOnlyRootFilesystem.** O pod corre com a raiz só-de-leitura; só
  `/tmp` (um `emptyDir`) é escrevível. Se o processo tentar escrever noutro sítio o
  arranque falha — monte um `emptyDir` para esse caminho no `values.yaml`.

## Terminado quando

- `kubectl rollout status` reporta o deployment completo.
- Todos os pods estão `Ready` e `/health/ready` devolve `200`.
- O HPA mostra o número de réplicas desejado dentro de `minReplicas`–`maxReplicas`:
  `kubectl get hpa seiva-seiva-server`.

---

*Parte da [documentação do Seiva](../index.md).*

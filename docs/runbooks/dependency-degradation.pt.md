# Runbook: degradação de dependência (circuit breaker aberto)

**Usar quando:** uma dependência a jusante (uma base de dados, um broker, a API de
outro serviço) está lenta ou a falhar, e o **circuit breaker do serviço disparou
para aberto**.

## O que significa um breaker aberto

Um [`CircuitBreaker`](https://github.com/ngolacloud/seiva/tree/main/crates/seiva-resilience)
envolve uma chamada a uma dependência. Após `failure_threshold` falhas
consecutivas passa a **Open** e **rejeita chamadas sem as executar**
(`BreakerError::Open`), largando carga da dependência para não a martelar enquanto
já está em baixo. Após `open_cooldown` admite um número limitado de chamadas de
sondagem (**HalfOpen**); sucessos suficientes fecham-no, qualquer falha reabre-o e
reinicia o cooldown.

```text
Closed ──(failure_threshold falhas)──▶ Open ──(cooldown)──▶ HalfOpen ──(sucessos)──▶ Closed
                                        ▲                        │
                                        └──(uma sondagem falha)──┘
```

Isto está a **funcionar como desenhado**: o breaker está a proteger o serviço e a
dependência. A sua tarefa é descobrir *porque é que a dependência está a falhar* e
restaurá-la — não desligar o breaker.

## Confirmar

Cada transição é registada (`tracing`) pelo breaker:

```console
$ kubectl logs -l app.kubernetes.io/name=seiva-server --tail=200 \
    | grep -i "circuit breaker"
# "circuit breaker tripped; shedding load off the dependency"  state=open
# "circuit breaker probing recovery"                           state=half_open
# "circuit breaker recovered"                                  state=closed
```

Se o breaker guarda uma dependência **crítica**, a sonda de readiness reflecte-o e
o pod é retirado de rotação:

```console
$ curl -s localhost:8080/health/ready
{"status":"unready","state":"dependency-unready"}   # HTTP 503
```

A liveness mantém-se `200` o tempo todo, por isso o Kubernetes **não** reinicia o
pod — um reinício não corrigiria uma falha a jusante e só deitaria fora o trabalho
em curso.

## Diagnosticar a dependência

O breaker é o sintoma; a dependência é a causa. Trabalhe de dentro para fora:

1. **Qual dependência?** A linha de log e o label da métrica do breaker nomeiam-na.
2. **É alcançável a partir do pod?**
   ```console
   $ kubectl exec <pod> -- true   # distroless não tem shell; use um debug container:
   $ kubectl debug <pod> -it --image=nicolaka/netshoot -- \
       sh -c 'curl -v http://<dependência>/health; nslookup <dependência>'
   ```
3. **A própria dependência está saudável?** Veja os dashboards/logs dela (ligações
   da BD esgotadas, broker em baixo, uma API parceira a devolver `5xx`).
4. **É timeout, não falha?** Se as chamadas estão lentas em vez de a falhar, o
   invólucro `timeout` está a converter lentidão em falhas que disparam o breaker. A
   dependência está sobrecarregada — olhe para a latência dela, não só para a taxa
   de erro.

## Agir

- **Se a dependência é o problema** (o caso habitual): corrija-a lá — escale-a,
  reinicie-a, faça failover, ou escale para o dono dela. **Não faça nada ao
  breaker.** Assim que a dependência recupera, a próxima sondagem do breaker tem
  sucesso e ele fecha automaticamente (observe a linha `state=closed`). Não é
  preciso deploy.
- **Se o breaker está mal afinado** (a disparar com ruído transitório normal, ou a
  demorar demasiado a recuperar), ajuste `BreakerConfig` na raiz de composição e
  role uma nova imagem — `failure_threshold`, `open_cooldown`, `success_threshold`.
  Isto é uma **mudança de código/configuração**, não uma acção de incidente;
  faça-a depois do incidente.
- **Se uma dependência não-crítica está em baixo** e quer o serviço a servir em
  modo degradado (o breaker a curto-circuitar para um fallback), confirme que o
  local de chamada tem um caminho de fallback; um breaker aberto sem fallback
  aparece como erro ao chamador, o que pode ser o comportamento correcto.

!!! warning "Não 'corrija' removendo o breaker"
    Desligar o breaker durante um incidente remove a única coisa que impede o
    serviço de martelar uma dependência já de rastos — transforma uma degradação
    contida numa falha em cascata. Corrija a dependência.

## Terminado quando

- O log do breaker mostra `state=closed` (recuperado), ou o dono da dependência
  confirma que esta está saudável de novo.
- `/health/ready` devolve `200` e os pods estão de volta à rotação.
- A taxa de erro e a latência para a dependência voltaram à linha de base no
  dashboard.

## Relacionado

- [Arranque e rollout](startup.md) — a separação readiness/liveness em que isto se apoia
- [Rollback](rollback.md) — se a degradação foi causada por uma versão má, não por uma dependência

---

*Parte da [documentação do Seiva](../index.md).*

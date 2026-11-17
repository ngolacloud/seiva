# Runbooks operacionais

São os procedimentos passo a passo para operar um serviço Seiva em produção. Cada
um é escrito para quem está de piquete: um sintoma, como o confirmar, o que fazer
e como saber que terminou.

Assumem que o serviço está implantado com o chart em
[`deploy/seiva-server`](https://github.com/ngolacloud/seiva/tree/main/deploy/seiva-server)
(um pod distroless não-root com sondas `/health/live` e `/health/ready`, um HPA e
um PodDisruptionBudget) e que usa as primitivas de resiliência em
[`seiva-resilience`](https://github.com/ngolacloud/seiva/tree/main/crates/seiva-resilience)
(circuit breaker, retry, timeout).

| Runbook | Quando recorrer a ele |
|---------|------------------------|
| [Arranque e rollout](startup.md) | A implantar uma nova versão, ou um pod que não fica `Ready` |
| [Degradação de dependência (breaker aberto)](dependency-degradation.md) | Uma dependência a jusante está a falhar; o circuit breaker disparou |
| [Rollback](rollback.md) | Uma versão está má e tem de ser revertida já |

## O modelo de saúde, num parágrafo

`/health/live` devolve `200` em **todos** os estados do ciclo de vida, incluindo
durante a drenagem — um pod vivo-mas-a-drenar deve ser retirado de rotação, não
morto a meio do encerramento. `/health/ready` devolve `200` apenas enquanto o
processo está `Ready` **e** a verificação de dependência crítica passa; `Starting`,
`Draining` ou uma dependência em falha devolvem `503` para o balanceador deixar de
o encaminhar. Um soluço transitório de uma dependência larga tráfego (readiness
`503`) sem desencadear uma tempestade de reinícios (a liveness mantém-se `200`).
Esta separação é a espinha dorsal dos três runbooks abaixo.

---

*Parte da [documentação do Seiva](../index.md).*

# Runbook: startup & rollout

**Use when:** you are deploying a new version, or a pod is stuck and will not
become `Ready`.

## Normal rollout

The chart uses a `RollingUpdate` with `maxUnavailable: 0` and `maxSurge: 1`: a new
pod must pass its probes before an old one is removed, so a healthy rollout never
dips below the running replica count.

```console
$ helm upgrade --install seiva deploy/seiva-server \
    --set image.tag=<new-version> --wait
$ kubectl rollout status deploy/seiva-seiva-server
```

A pod's lifecycle, as the probes see it:

1. **`startupProbe`** polls `/health/live` until the process is accepting
   connections. Until it passes, liveness and readiness are not yet consulted, so
   a slow boot is never mistaken for a crash loop.
2. **`readinessProbe`** (`/health/ready`) stays `503` while the process is
   `Starting` and while its critical-dependency check fails, so no traffic is
   routed until the service can actually serve it. The composition root calls
   `Health::set_ready()` once wiring is complete.
3. **`livenessProbe`** (`/health/live`) is `200` from the moment the process is up.

## A pod will not become Ready

Confirm what the probes are reporting:

```console
$ kubectl get pods -l app.kubernetes.io/name=seiva-server
$ kubectl describe pod <pod>          # look at the Events and probe failures
$ kubectl port-forward <pod> 8080     # then, in another shell:
$ curl -s -o /dev/null -w '%{http_code}\n' localhost:8080/health/live
$ curl -s localhost:8080/health/ready  # body names WHY it is unready
```

Decide from the two probes:

| `/health/live` | `/health/ready` | Meaning | Action |
|----------------|-----------------|---------|--------|
| `200` | `503` `"state":"starting"` | Still wiring up | Wait; if it never flips, the composition root never called `set_ready()` — check boot logs for a panic/hang before readiness is set |
| `200` | `503` `"state":"dependency-unready"` | Up, but a critical dependency is unreachable | This is **dependency degradation** — see [that runbook](dependency-degradation.md) |
| connection refused | — | Process not listening | Check `SEIVA_HTTP_ADDR` is `0.0.0.0:<port>` (the chart sets this); a `127.0.0.1` bind is unreachable from the probe |
| crash loop | — | Boot fails | `kubectl logs <pod> --previous`; a config error exits non-zero with one JSON line naming the bad variable |

## Common boot failures

- **Invalid configuration.** `seiva-server` validates its environment at startup
  and exits non-zero, logging each problem. `kubectl logs <pod> --previous` shows
  the offending variable (e.g. a missing `SEIVA_OIDC_ISSUER` when the OIDC trio is
  partially set). Fix the ConfigMap/Secret and re-roll.
- **`AppEnv` safety guard.** Enabling a dev shortcut (e.g.
  `SEIVA_EXPOSE_ERROR_DETAILS`) while `APP_ENV=production` is a **startup error**,
  by design. Remove the shortcut or do not run that image in production.
- **readOnlyRootFilesystem write.** The pod runs with a read-only root; only
  `/tmp` (an `emptyDir`) is writable. If the process tries to write elsewhere the
  boot fails — mount an `emptyDir` for that path in `values.yaml`.

## Done when

- `kubectl rollout status` reports the deployment complete.
- Every pod is `Ready` and `/health/ready` returns `200`.
- The HPA shows the desired replica count within `minReplicas`–`maxReplicas`:
  `kubectl get hpa seiva-seiva-server`.

---

*Part of the [Seiva documentation](../index.md).*

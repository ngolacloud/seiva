# Runbook: rollback

**Use when:** a release is bad — new errors, broken readiness, a regression — and
must be reverted **now**. Roll back first, diagnose afterwards.

## Decide: roll back or roll forward?

Roll **back** when the previous version was known-good and the fix is not
immediate. Roll **forward** only when the fix is trivial, already built, and
faster to ship than a revert. When in doubt, roll back — it is the reversible
move.

## Roll back with Helm

Helm keeps a revision history; one command reverts the whole release (image,
config, everything) to the previous revision.

```console
$ helm history seiva                      # find the last good REVISION
$ helm rollback seiva <revision> --wait   # omit <revision> to go back exactly one
$ kubectl rollout status deploy/seiva-seiva-server
```

The same `RollingUpdate` guarantees apply in reverse: the good version's pods must
pass their probes before the bad ones are removed, so the rollback never drops the
service below its replica count. The `PodDisruptionBudget` (`minAvailable: 1`)
keeps at least one replica serving throughout.

## Roll back with kubectl (no Helm)

If the change was applied with `kubectl` rather than Helm:

```console
$ kubectl rollout history deploy/seiva-seiva-server
$ kubectl rollout undo deploy/seiva-seiva-server          # previous revision
$ kubectl rollout undo deploy/seiva-seiva-server --to-revision=<n>
$ kubectl rollout status deploy/seiva-seiva-server
```

## Graceful by construction

A rollback is safe to run mid-traffic because shutdown is graceful:

- On `SIGTERM` the pod flips readiness to `Draining` (`/health/ready` → `503`), so
  the load balancer stops sending it new requests, while `/health/live` stays
  `200` so Kubernetes does not kill it mid-drain.
- In-flight requests finish within `terminationGracePeriodSeconds` (default `30`).
- Background workers observe the same shutdown signal and stop after their current
  pass, so no work is interrupted halfway.

## If the rollback itself will not come up

Treat it as a startup failure: follow the [startup runbook](startup.md). The most
common cause is a **config/schema mismatch** — the bad release migrated a database
or changed a ConfigMap shape that the old binary cannot read. In that case a
code/image rollback alone is not enough; you must also revert the incompatible
config or data change. This is why **backward-compatible migrations** matter: a
release you cannot roll back is not really deployed, only gambled.

## After the service is stable

- Capture the bad image tag and the failing symptom before anyone redeploys.
- Keep the bad release's pod logs: `kubectl logs <bad-pod> --previous`.
- Open the postmortem; the fix goes through the normal rollout, not a hotfix into
  the running pods.

## Done when

- `helm history seiva` (or `kubectl rollout history`) shows the good revision as
  current.
- `kubectl rollout status` reports complete.
- `/health/ready` is `200` on every pod and error rate is back to baseline.

## Related

- [Startup & rollout](startup.md) · [Dependency degradation](dependency-degradation.md)

---

*Part of the [Seiva documentation](../index.md).*

# Deployment selection

Choose the artifact boundary before writing platform configuration.

| Deployment target | Build artifact | Start with |
| --- | --- | --- |
| Linux process supervisor | Release executable and five sidecars | [Production operations](production.md#build-admission) |
| Docker or another OCI runtime | OCI image plus the external release sidecars | [Container image](../guides/container-image.md) |
| One-shot Wasm Component | Release executable and five sidecars | [Component tasks](component-tasks.md) |

Production application executables support `linux-x64` and `linux-arm64`.
The build host does not need to match: `tysel build --target` packages with a
verified same-version official target runtime. Tysel does not compile its
native runtime for arbitrary target triples.

## Platform boundary

Tysel produces an executable or container context. It does not generate:

- systemd units;
- Kubernetes resources or Helm charts;
- ingress, TLS, DNS, or load-balancer configuration;
- secret-manager objects;
- registry authentication or image signatures.

Those resources belong to the deployment platform. Apply the runtime contract
below when creating them.

## Required runtime contract

| Concern | Requirement |
| --- | --- |
| Identity | Run as a dedicated unprivileged account. The generated container uses `65532:65532`. |
| Listener | Use the address embedded by the build manifest. Containers require `0.0.0.0` or `[::]`. |
| Working directory | Keep it stable; relative filesystem and SQLite paths resolve from it at runtime. |
| Writable storage | Grant only manifest-declared filesystem roots and the selected SQLite path. |
| Secrets | Inject declared secret and database environment variables at runtime. |
| Health | Probe an application-owned HTTP route; Tysel does not add one. |
| Shutdown | Send the normal termination signal and allow at least the request timeout. |
| Promotion | Deploy the admitted executable or image digest, not a mutable rebuild. |

## Artifact boundary

For a direct executable deployment, keep these files together in the artifact
store:

```text
APP
APP.sha256
APP.compat.json
APP.sbom.cdx.json
APP.licenses.json
APP.evidence.json
```

For the `service` profile, the runtime host needs only `APP`. An `isolated`
application also requires a matching-version, matching-platform `tysel-worker`
beside the executable, or its path set through `TYSEL_WORKER`. The build command
reports this requirement but does not copy the worker into the output directory.
Keep the worker alongside the application in the deployment artifact set;
copying only the isolated application will fail at startup with
`worker binary not found`. Admission and rollback still depend on the
complete immutable set. When using a container, record the mapping from the
image digest to the admitted executable digest.

## Next step

- Build or generate an OCI image with [Container image](../guides/container-image.md).
- Preserve and promote immutable identities with
  [Continuous delivery](continuous-delivery.md).
- Sign and verify the artifact set with [Application release](../guides/reproducible-release.md).
- Apply health, backup, rollout, monitoring, and incident requirements from
  [Production operations](production.md).

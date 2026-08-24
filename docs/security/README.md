# Security model

Tysel treats host authority as an explicit contract. Network destinations,
secret names, database grants, filesystem roots, limits, and execution profile
are declared in the versioned TOML or JSON application manifest and enforced
by the native host.

## What Tysel protects

- undeclared host resources are denied;
- JavaScript receives opaque handles instead of raw secret values;
- database and filesystem access is limited to named grants and pinned roots;
- request size, response size, memory, CPU turns, deadlines, and concurrency
  are bounded;
- isolated JavaScript runs outside the service process;
- durable effects and signals are persisted for recovery and auditability.

The effective authority is the intersection of four inputs:

```text
manifest grants ∩ deployment policy ∩ execution profile ∩ runtime support
```

Adding a permission to the manifest cannot bypass a restriction imposed by an
execution profile or deployment.

## Trust boundaries

The `service` profile is for trusted first-party code. Its JavaScript isolate
is capability-restricted, but it shares a process with the native host.

The `isolated` profile moves application execution to `tysel-worker` and denies
host-facing capabilities. On Linux, workers add Landlock, seccomp, resource
limits, and best-effort cgroup memory enforcement. Supervisor recovery replaces
a crashed worker. macOS isolation supports development checks only and is not
the production sandbox gate.

The experimental `component` profile runs a Wasm Component in Wasmtime under
`isolated-task` trust mode. It receives an empty WASI context and no application
capability by default. Effective filesystem access requires the guest import,
manifest root, and deployment policy to intersect. Memory, fuel, time, input,
output, and error payloads are bounded. This is not a promise to run arbitrary
WASI applications safely or compatibly.

Tysel does not run arbitrary native addons or binaries inside this boundary.
Subprocesses, dynamic libraries, Node.js builtins, and ambient OS access are
outside the application contract.

## Deployment responsibilities

Tysel does not replace operating-system hardening or network policy.
Production operators remain responsible for:

- running the executable as a dedicated non-root identity;
- terminating public TLS at a maintained ingress or reverse proxy;
- constraining outbound traffic at the infrastructure layer;
- protecting environment variables, databases, signing keys, and durable
  stores;
- applying host updates and retaining release evidence;
- monitoring denials, timeouts, worker crashes, and recovery behavior.

Use Linux for production isolated workloads. Validate the exact artifact and
configuration in the target environment with `tysel doctor`, `tysel check`,
`tysel inspect`, and the application test suite.

## Reporting vulnerabilities

Do not open a public issue containing exploit details or secrets. Use the
repository's private security reporting channel when available.

Continue with the [capability matrix](../capabilities/README.md),
[execution profiles](../concepts/execution-profiles.md), and
[Wasm Component capabilities](../reference/component/capabilities.md), or the
[production operations](../operations/production.md).

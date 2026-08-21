# Security

Tysel denies undeclared host resources. The service profile is capability-restricted;
the isolated profile additionally separates application execution into a worker process
and denies host-facing capabilities. On Linux, isolated workers add Landlock, seccomp,
and best-effort cgroup memory enforcement. macOS is not the production isolation gate.

See the [capability matrix](../capabilities/README.md),
[process-isolation decision](../adr/006-process-isolation.md), and
[production operations runbook](../operations/production.md) for the current security
contract and operating requirements.

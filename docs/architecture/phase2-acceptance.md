# Phase 2 acceptance

Phase 2 is the developer-experience milestone after Production v1. It is complete when a new user can discover, create, validate, test, debug, and package an application without reading runtime internals.

| Area | Acceptance contract | Automated evidence |
| --- | --- | --- |
| Project creation | `init` creates source, manifest, tests, package scripts, and ignores; conflicts cause no partial writes. | CLI integration tests run generated `check` and `test`. |
| Compatibility | Four-state classification; shim precedence; JSON schema; policy exit codes. | Human/JSON/strict integration tests. |
| Debugging | Fatal CLI and HTTP errors are structured; runtime and test frames map to original TypeScript. | JSON parsing, preserved JavaScript stacks, and HTTP/test source-map assertions. |
| Tests | `test()` plus assertions, async cases, engine-enforced per-test isolation and timeout, JSON/human reports. | Passing template and mixed pass/fail/synchronous-loop/continuation suites. |
| Images | ELF64 type, architecture and interpreter gate; wildcard listener; non-root image; inspectable context; overwrite protection. | Dockerfile/context integration tests without a Docker daemon. |
| Documentation | Quick start, CLI, runtime API, capabilities, compatibility, and operations are linked from one hub. | Local-link validation in the release checklist. |

Phase 2 deliberately does not add a Node compatibility layer, an interactive Chrome-style debugger, outbound WebSocket, HTTP/2, or a hosted cloud platform.

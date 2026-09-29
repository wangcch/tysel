// Identical code in service and isolated profiles. Build time is not timed.
import type { TyselApp } from "@tysel/types";
import { triageEnvelope } from "../../examples/isolated-plugin/src/triage.js";

export default {
  fetch(request) { return triageEnvelope(request); },
} satisfies TyselApp;

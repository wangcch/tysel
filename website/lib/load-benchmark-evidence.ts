import type { AdmissionBoard } from "./benchmark-evidence";
import { buildAdmissionBoard } from "./build-admission-board";
import { loadBenchmarkSnapshot } from "./load-benchmark-snapshot";

export { buildAdmissionBoard } from "./build-admission-board";

/** Server-only: reads the checked-in / CI-synced evidence snapshot. */
export function loadAdmissionBoard(): AdmissionBoard {
  return buildAdmissionBoard(loadBenchmarkSnapshot());
}

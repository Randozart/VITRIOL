// PROVENANCE: original Officina opportunistic-preparation admission policy.
// Offline qualification policy; no runtime scheduler invokes this yet.
// Admission is not CUDA preemption or a measured foreground-latency guarantee.
export interface PreparationBudget {
  device: "cpu" | "gpu";
  modelIdentity: string;
  tokenizerIdentity: string;
  schemaIdentity: string;
  resident: boolean;
  boundedWorkMs: number;
  allowedWorkMs: number;
  workingBytes: number;
  reserveBytes: number;
  allowDecodeOverlap: boolean;
}
export interface PreparationFacts {
  now: number;
  sampledAt: number;
  foregroundPrefill: boolean | null;
  queuedRequests: number | null;
  foregroundDecode: boolean | null;
  cpuLoad: number | null;
  hostFreeBytes: number | null;
  gpu?: { sampledAt: number; utilization: number; freeBytes: number };
}
export interface Admission { allowed: boolean; reason: string }

const deny = (reason: string): Admission => ({ allowed: false, reason });

function fresh(now: number, sampledAt: number): boolean {
  return Number.isFinite(sampledAt) && sampledAt <= now && now - sampledAt <= 2000;
}

/** Qualification is supplied by an operator benchmark, not inferred from utilization. */
function qualifiedBudget(budget: PreparationBudget): boolean {
  const identities = [budget.modelIdentity, budget.tokenizerIdentity, budget.schemaIdentity];
  if (!identities.every((identity) => typeof identity === "string" && identity.trim().length > 0)) return false;
  const quantities = [budget.boundedWorkMs, budget.allowedWorkMs, budget.workingBytes, budget.reserveBytes];
  if (!quantities.every(Number.isFinite)) return false;
  return budget.boundedWorkMs > 0 && budget.boundedWorkMs <= budget.allowedWorkMs && budget.workingBytes > 0 && budget.reserveBytes >= 0;
}

function cpuAdmission(budget: PreparationBudget, facts: PreparationFacts): Admission {
  if (facts.cpuLoad === null || !Number.isFinite(facts.cpuLoad) || facts.cpuLoad < 0 || facts.cpuLoad > 0.5) return deny("CPU busy or unknown");
  if (facts.hostFreeBytes === null || !Number.isFinite(facts.hostFreeBytes) || facts.hostFreeBytes < budget.workingBytes + budget.reserveBytes) return deny("insufficient or unknown host headroom");
  return { allowed: true, reason: "CPU capacity admitted" };
}

function gpuAdmission(budget: PreparationBudget, facts: PreparationFacts): Admission {
  if (!budget.resident) return deny("GPU preparation cannot cold-load or replace a model");
  const gpu = facts.gpu;
  if (!gpu || !fresh(facts.now, gpu.sampledAt)) return deny("GPU facts unavailable or stale");
  if (!Number.isFinite(gpu.utilization) || gpu.utilization < 0 || gpu.utilization > 0.5) return deny("GPU has insufficient compute slack");
  if (!Number.isFinite(gpu.freeBytes) || gpu.freeBytes < budget.workingBytes + budget.reserveBytes) return deny("insufficient GPU headroom");
  return { allowed: true, reason: facts.foregroundDecode ? "qualified half-idle GPU capacity admitted" : "idle GPU capacity admitted" };
}

/** Unknown/stale resources deny preparation; half-idle still needs its own memory budget. */
export function admitPreparation(budget: PreparationBudget, facts: PreparationFacts): Admission {
  if (!qualifiedBudget(budget)) return deny("unqualified work, identity or memory budget");
  if (!Number.isFinite(facts.now) || !fresh(facts.now, facts.sampledAt)) return deny("stale preparation facts");
  if (facts.foregroundPrefill !== false || facts.queuedRequests !== 0 || typeof facts.foregroundDecode !== "boolean") return deny("foreground work pending or unknown");
  if (facts.foregroundDecode && !budget.allowDecodeOverlap) return deny("decode overlap is disabled");
  if (budget.device === "cpu") return cpuAdmission(budget, facts);
  if (budget.device === "gpu") return gpuAdmission(budget, facts);
  return deny("unknown preparation device");
}

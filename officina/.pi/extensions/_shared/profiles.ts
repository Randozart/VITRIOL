// Routing profiles: auto / fast / deep / local.

import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export type RoutingProfile = "auto" | "fast" | "deep" | "local";

export function profilePath(): string {
  return process.env.OFFICINA_ROUTING_PROFILE_FILE || join(homedir(), ".vitriol", "officina", "routing-profile");
}

export function readProfile(): RoutingProfile {
  const env = (process.env.OFFICINA_ROUTING_PROFILE || "").trim().toLowerCase();
  if (env === "auto" || env === "fast" || env === "deep" || env === "local") return env;
  try {
    const raw = readFileSync(profilePath(), "utf8").trim().toLowerCase();
    if (raw === "auto" || raw === "fast" || raw === "deep" || raw === "local") return raw;
  } catch {
    // default
  }
  return "auto";
}

export function writeProfile(profile: RoutingProfile): void {
  mkdirSync(dirname(profilePath()), { recursive: true, mode: 0o700 });
  const tmp = `${profilePath()}.tmp-${process.pid}`;
  writeFileSync(tmp, `${profile}\n`, { mode: 0o600 });
  chmodSync(tmp, 0o600);
  renameSync(tmp, profilePath());
  chmodSync(profilePath(), 0o600);
}

export function profileAllowsCloud(profile: RoutingProfile): boolean {
  return profile !== "local";
}

/** Family rank for a profile; lower is preferred. */
export function profileFamilyRank(profile: RoutingProfile, id: string): number {
  const s = id.toLowerCase();
  if (profile === "fast") {
    if (/deepseek.*v4\.1.*flash/.test(s)) return 0;
    if (/deepseek.*flash/.test(s)) return 1;
    if (/mimo.*flash/.test(s)) return 2;
    if (/glm.*flash/.test(s)) return 3;
    if (/flash/.test(s)) return 4;
    return 5;
  }
  if (profile === "deep") {
    if (/agentrouter\/(gpt|claude|opus)/.test(s)) return 0;
    if (/deepseek.*v4\.1.*flash/.test(s)) return 1;
    if (/deepseek.*flash/.test(s)) return 2;
    if (/mimo.*flash/.test(s)) return 3;
    if (/glm.*flash/.test(s)) return 4;
    return 5;
  }
  if (/deepseek.*v4\.1.*flash/.test(s)) return 0;
  if (/deepseek.*flash/.test(s)) return 1;
  if (/mimo.*flash/.test(s)) return 2;
  if (/glm.*flash/.test(s)) return 3;
  if (/flash/.test(s)) return 4;
  return 5;
}

/** Stable sort; auto preserves the catalog/free ranking. */
export function sortIdsForProfile<T extends { id: string }>(items: T[], profile: RoutingProfile): T[] {
  if (profile === "auto") return [...items];
  return [...items].sort((a, b) => {
    const d = profileFamilyRank(profile, a.id) - profileFamilyRank(profile, b.id);
    return d !== 0 ? d : a.id.localeCompare(b.id);
  });
}

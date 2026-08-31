// verify-contract — config-file validation. Pure module: the JSON check
// (including duplicate-key detection, which JSON.parse silently forgives)
// runs in-process; TOML/YAML delegate to python3 when the module exists,
// via an injected run layer so this file stays testable.
//
// Parsing a config file is trivial for an algorithm and a real failure
// mode for a model. Output is a single error string per file; clean files
// cost zero tokens.
//
// Provenance: original work, this repo (First-Party Mandate). Plan:
// .opencode/plans/officina-algorithmic-support-2026-08-31.md (P4).

export interface ContractConfig {
  enabled: boolean;
  toml: boolean;
  yaml: boolean;
}

export function contractConfig(env: NodeJS.ProcessEnv = process.env): ContractConfig {
  return {
    enabled: env.OFFICINA_NO_CONTRACT !== "1",
    toml: env.OFFICINA_CONTRACT_TOML === "1",
    yaml: env.OFFICINA_CONTRACT_YAML === "1",
  };
}

/**
 * Validate a JSON string. Returns an error message, or null when valid.
 * Detects duplicate keys (last-one-wins in JSON.parse, but almost always
 * a merge accident worth flagging) and parse errors. Pure.
 */
export function validateJson(text: string): string | null {
  try {
    JSON.parse(text);
  } catch (err) {
    return `invalid JSON: ${(err as Error).message.slice(0, 200)}`;
  }
  return duplicateKeys(text);
}

/**
 * Scan raw JSON text for duplicate keys per object using a brace-tracking
 * tokenizer. Returns an error message listing the first offenders, or null.
 * Pure — handles strings/escapes; arrays are transparent. Best-effort:
 * JSON.parse has already ruled on validity before this runs.
 */
export function duplicateKeys(text: string): string | null {
  const keyStack: Array<Map<string, number>> = [];
  const inArray: boolean[] = [];
  let i = 0;
  const offenders = new Map<string, number>();
  while (i < text.length) {
    const ch = text[i];
    if (ch === '"') {
      let j = i + 1;
      let raw = "";
      while (j < text.length) {
        if (text[j] === "\\") {
          raw += text.slice(j, j + 2);
          j += 2;
          continue;
        }
        if (text[j] === '"') break;
        raw += text[j];
        j++;
      }
      let k = j + 1;
      while (k < text.length && /\s/.test(text[k])) k++;
      const inObj = keyStack.length > 0 && !inArray[inArray.length - 1];
      if (text[k] === ":" && inObj) {
        const top = keyStack[keyStack.length - 1];
        const n = (top.get(raw) ?? 0) + 1;
        top.set(raw, n);
        if (n === 2) offenders.set(raw, (offenders.get(raw) ?? 0) + 1);
      }
      i = j + 1;
      continue;
    }
    if (ch === "{") {
      keyStack.push(new Map());
      inArray.push(false);
    } else if (ch === "}") {
      keyStack.pop();
      inArray.pop();
    } else if (ch === "[") {
      keyStack.push(new Map());
      inArray.push(true);
    } else if (ch === "]") {
      keyStack.pop();
      inArray.pop();
    }
    i++;
  }
  if (offenders.size === 0) return null;
  const names = [...offenders.keys()].slice(0, 5).join(", ");
  return `duplicate JSON key(s): ${names} — last-one-wins hides the other value; remove the duplicate`;
}

/** Route a config file to its validator kind, or null. Pure. */
export function contractKindFor(file: string, cfg: ContractConfig): "json" | "toml" | "yaml" | null {
  if (/\.jsonc?$/.test(file)) return "json";
  if (file.endsWith(".toml") && cfg.toml) return "toml";
  if (/\.(ya?ml)$/.test(file) && cfg.yaml) return "yaml";
  return null;
}

/** External (python3) validators. kind: "toml" | "yaml". Returns error or null. */
export function runExternalContract(
  kind: "toml" | "yaml",
  file: string,
  run: (argv: string[]) => { ok: boolean; output: string },
): string | null {
  const code =
    kind === "toml"
      ? `import tomllib,sys; tomllib.load(open(sys.argv[1],'rb'))`
      : `import yaml,sys; yaml.safe_load(open(sys.argv[1]))`;
  const res = run(["python3", "-c", code, file]);
  if (res.ok) return null;
  return `invalid ${kind.toUpperCase()}: ${res.output.slice(0, 200)}`;
}

// Composer prediction runtime (2026-11-16): the staged suggestion must follow
// the WORK, never the rulebook. Mocks the lane + task source so the wiring
// (task → predict → suggestion file) is what is under test.
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const harness = vi.hoisted(() => ({
  tasks: [] as Array<{ id: number; description: string; status: string }>,
}));

// The extension resolves its state directory at import time, so the isolated
// HOME must be in place BEFORE the module loads (2026-11-16: importing first
// wrote a mock suggestion into the operator's real file).
const harnessHome = mkdtempSync(join(tmpdir(), "officina-suggest-home-"));
process.env.HOME = harnessHome;

vi.mock("../caduceus/catalog.ts", () => ({
  extractCatalogModels: (value: unknown) => value,
  freeToolCandidates: () => [{ id: "test/free" }],
}));
vi.mock("../_shared/budget.ts", () => ({
  budgetStatus: () => ({ allowed: true }),
}));
vi.mock("../auto-route/classifier.ts", () => ({
  classifyTurn: () => ({ privacy: "safe", complexity: 0.1 }),
}));
vi.mock("../task-state/index.ts", () => ({
  getTaskItems: () => harness.tasks,
}));

process.env.HOME = harnessHome;
let install: (pi: ExtensionAPI) => void;

let home: string;
let cwd: string;
let handlers: Map<string, (...args: unknown[]) => unknown>;
let fetcher: ReturnType<typeof vi.fn>;

const fire = (event: string, payload: unknown = {}, ctx?: unknown) =>
  handlers.get(event)?.(payload, ctx);
const suggestionPath = () => join(home, ".vitriol", "officina", "prompt-suggestion.json");
const suggestion = () =>
  JSON.parse(
    readFileSync(
      suggestionPath(),
      "utf8",
    ),
  ) as { text: string; source: string };
const reply = (content: string) =>
  new Response(JSON.stringify({ choices: [{ message: { content } }] }));

/** Drive one finished turn: the operator asked for something, the agent replied. */
async function turn(userText: string): Promise<void> {
  // session_start stages a "cleared" doc; drop it so only this turn's staging
  // can write the file we then assert on.
  rmSync(suggestionPath(), { force: true });
  fire("message_end", {
    message: { role: "user", content: [{ type: "text", text: userText }] },
  });
  fire("message_end", {
    message: {
      role: "assistant",
      content: [{ type: "text", text: "done: wired the handler" }],
    },
  });
  fire("agent_end");
  // Staging is detached: wait for the lane call, then for a settled file
  // (a cleared doc or a real suggestion).
  await vi.waitFor(() => expect(fetcher).toHaveBeenCalled());
  await vi.waitFor(() => {
    const doc = suggestion();
    expect(typeof doc.text).toBe("string");
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeAll(async () => {
  ({ default: install } = await import("./index.ts"));
});

beforeEach(() => {
  home = harnessHome;
  cwd = mkdtempSync(join(tmpdir(), "officina-suggest-cwd-"));
  process.env.HOME = home;
  // The lane picker reads the catalog cache; a present-but-empty cache with the
  // mocked extractor is enough to select the test lane.
  mkdirSync(join(home, ".vitriol", "officina"), { recursive: true });
  writeFileSync(
    join(home, ".vitriol", "officina", "caduceus-models.json"),
    "{}",
  );
  vi.stubEnv("OFFICINA_CADUCEUS_API_KEY", "test");
  vi.stubEnv("OFFICINA_NO_PREDICTIONS", "0");
  vi.stubEnv("OFFICINA_NO_PROMPT_SUGGEST", "0");
  harness.tasks = [];
  handlers = new Map();
  fetcher = vi.fn().mockResolvedValue(reply("SKIP"));
  vi.stubGlobal("fetch", fetcher);
  install({
    on: (event: string, handler: (...args: unknown[]) => unknown) =>
      handlers.set(event, handler),
    registerTool: () => {},
    registerCommand: () => {},
  } as unknown as ExtensionAPI);
  fire(
    "session_start",
    {},
    {
      cwd,
      hasUI: false,
      ui: {},
      sessionManager: { getSessionId: () => "s-1" },
    },
  );
});

afterEach(() => {
  fire("session_shutdown");
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  process.env.HOME = harnessHome;
  rmSync(cwd, { recursive: true, force: true });
  rmSync(join(home, ".vitriol"), { recursive: true, force: true });
});

describe("composer prediction runtime", () => {
  it("recommends the task in progress, not harness bookkeeping", async () => {
    harness.tasks = [
      { id: 1, description: "wire the Esc handler", status: "in_progress" },
    ];
    await turn("esc doesn't properly end the work");
    const doc = suggestion();
    expect(doc.text).toContain("▸ next: wire the Esc handler");
    expect(doc.source).toBe("heuristic");
    // The lane is told what the operator asked for, and is forbidden the
    // rulebook echo (request-aligned signal ordering).
    const prompt = String(fetcher.mock.calls[0]?.[1]?.body ?? "");
    expect(prompt).toContain("task in progress: wire the Esc handler");
    expect(prompt).toContain(
      "operator's last request: esc doesn't properly end the work",
    );
  });

  it("clears instead of inventing advice when there is no task (D4)", async () => {
    await turn("what's next?");
    expect(suggestion().text).toBe("");
  });

  it("rejects an instruction-shaped draft (N3)", async () => {
    fetcher.mockResolvedValue(reply("commit the work (95 changed files)"));
    await turn("what's next?");
    expect(suggestion().text).toBe("");
  });

  it("keeps a work-shaped draft when there is no task", async () => {
    fetcher.mockResolvedValue(reply("check the KV restore path"));
    await turn("what's next?");
    expect(suggestion().text).toBe("check the KV restore path");
    expect(suggestion().source).toBe("llm");
  });
});

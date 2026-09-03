import { describe, it, expect } from "vitest";
import { renderTaskBlock, taskStateConfig, validateTasks, type TaskItem } from "./state.ts";
import { lastCopyIsCurrent, sessionFileStem } from "./index.ts";

const t = (over: Partial<TaskItem>): TaskItem => ({
  id: 1,
  description: "do the thing",
  status: "pending",
  ...over,
});

describe("taskStateConfig", () => {
  it("defaults and kill switch", () => {
    const cfg = taskStateConfig({});
    expect(cfg.enabled).toBe(true);
    expect(cfg.maxItems).toBe(15);
    expect(cfg.dir).toBe(".officina/tasks"); // branding shim 2026-09-02
    expect(taskStateConfig({ TRIS_NO_TASK_STATE: "1" }).enabled).toBe(false);
  });
});

describe("validateTasks", () => {
  it("accepts a clean list and defaults missing ids to position", () => {
    const v = validateTasks([
      { description: "a", status: "pending" },
      { id: 7, description: "b", status: "completed" },
    ]);
    expect(v.error).toBeUndefined();
    expect(v.tasks?.[0].id).toBe(1);
    expect(v.tasks?.[1].id).toBe(7);
  });

  it("rejects non-arrays, empty descriptions, unknown statuses", () => {
    expect(validateTasks("nope").error).toContain("array");
    expect(validateTasks([{ status: "pending" }]).error).toContain("description");
    expect(validateTasks([{ description: "x", status: "flying" }]).error).toContain("flying");
  });

  it("caps file size at 40", () => {
    const many = Array.from({ length: 41 }, (_, i) => ({ description: `t${i}`, status: "pending" }));
    expect(validateTasks(many).error).toContain("too many");
  });

  it("truncates overlong descriptions (budget guard)", () => {
    const v = validateTasks([{ description: "z".repeat(500), status: "pending" }]);
    expect((v.tasks?.[0].description ?? "").length).toBe(200);
  });
});

describe("renderTaskBlock", () => {
  it("renders the R2.4 checklist shape with progress footer", () => {
    const block = renderTaskBlock([
      t({ id: 1, status: "completed" }),
      t({ id: 2, status: "in_progress" }),
      t({ id: 3, status: "pending" }),
    ]);
    expect(block).toContain("[x] 1.");
    expect(block).toContain("[>] 2.");
    expect(block).toContain("[ ] 3.");
    expect(block).toContain("1/3 done, 2 open");
  });

  it("caps at 15 items with [+N more]", () => {
    const items = Array.from({ length: 20 }, (_, i) => t({ id: i + 1, description: `step ${i + 1}` }));
    const block = renderTaskBlock(items, 15);
    expect(block).toContain("step 15");
    expect(block).not.toContain("step 16");
    expect(block).toContain("[+5 more]");
  });

  it("empty list renders empty block (injection skipped)", () => {
    expect(renderTaskBlock([])).toBe("");
  });

  it("fits the ~200-token budget on a full 15-item list", () => {
    const items = Array.from({ length: 15 }, (_, i) => t({ id: i + 1, description: `do step ${i + 1} carefully` }));
    const block = renderTaskBlock(items, 15);
    expect(Math.ceil(block.length / 3.5)).toBeLessThanOrEqual(200);
  });
});

describe("lastCopyIsCurrent (cache-safe dedupe)", () => {
  const copy = (content: string) => ({ role: "custom", customType: "lc-tasks", content });

  it("skips when the last copy carries the identical block", () => {
    const msgs = [copy("B"), { role: "assistant" }, copy("B")];
    expect(lastCopyIsCurrent(msgs, "B")).toBe(true);
  });

  it("injects when state changed since the last copy", () => {
    const msgs = [copy("B"), { role: "assistant" }];
    expect(lastCopyIsCurrent(msgs, "C")).toBe(false);
  });

  it("injects when no copy exists yet", () => {
    expect(lastCopyIsCurrent([{ role: "user" }], "B")).toBe(false);
  });
});

describe("sessionFileStem", () => {
  it("maps session files to task filenames", () => {
    expect(sessionFileStem("/x/y/.pi/sessions/2026-08-29T12-00-00.jsonl")).toBe("2026-08-29T12-00-00");
    expect(sessionFileStem(null)).toBe("default");
  });
});

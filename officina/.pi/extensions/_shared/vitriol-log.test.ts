import { describe, expect, it } from "vitest";
import { parseVitriolLog } from "./vitriol-log.ts";

describe("vitriol log parser", () => {
  it("tracks the latest task through prefill and decode", () => {
    const text = [
      "22.13.146.561 I slot print_timing: id  0 | task 5860 | prompt processing, n_tokens =   1024, progress = 0.85, t =   3.07 s / 333.94 tokens per second",
      "22.17.002.658 I slot print_timing: id  0 | task 5860 | prompt processing, n_tokens =   2432, progress = 0.88, t =   6.71 s / 362.51 tokens per second",
      "22.41.691.584 I slot print_timing: id  0 | task 5860 | n_gen =    284, tg =  28.30 t/s, tg_3s =  30.80 t/s",
      "22.42.406.331 I slot print_timing: id  0 | task 5860 | prompt eval time =   21734.31 ms /  7469 tokens (    2.91 ms per token,   343.65 tokens per second)",
      "22.42.406.336 I slot print_timing: id  0 | task 5860 |        eval time =   10715.13 ms /   308 tokens (   34.90 ms per token,    28.65 tokens per second)",
      "22.42.406.355 I slot print_timing: id  0 | task 5860 | draft acceptance = 0.80370 (  217 accepted /   270 generated), mean len =  3.41",
    ].join("\n");
    const snap = parseVitriolLog(text)!;
    expect(snap.task).toBe(5860);
    expect(snap.phase).toBe("decode");
    expect(snap.generated).toBe(284);
    expect(snap.tg).toBe(28.3);
    expect(snap.tg3s).toBe(30.8);
    expect(snap.prefillTps).toBe(362.51);
    expect(snap.evalTps).toBe(28.65);
    expect(snap.draftAcceptance).toBeCloseTo(0.8037);
    expect(snap.draftMeanLen).toBe(3.41);
  });

  it("returns null for unrelated text", () => {
    expect(parseVitriolLog("engine started\n")).toBeNull();
  });
});

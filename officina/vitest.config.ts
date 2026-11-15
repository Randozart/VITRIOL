import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Bound SDK/jiti startup contention while local inference shares the host.
    maxWorkers: 4,
  },
});

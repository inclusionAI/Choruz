import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["lib/**/*.test.ts", "components/**/*.test.ts", "app/**/*.test.ts", "plugins/**/*.test.ts"],
    environment: "node",
  },
});

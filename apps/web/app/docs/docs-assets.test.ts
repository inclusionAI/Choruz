import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const docsRoot = fileURLToPath(new URL(".", import.meta.url));
const publicRoot = fileURLToPath(new URL("../../public/", import.meta.url));

function pagesIn(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return pagesIn(path);
    return entry.name === "page.tsx" ? [path] : [];
  });
}

describe("documentation assets", () => {
  it("keeps every local screenshot reference backed by a public file", () => {
    const missing = pagesIn(docsRoot).flatMap((page) => {
      const source = readFileSync(page, "utf8");
      return [...source.matchAll(/src=["']\/(docs-img\/[^"']+)["']/gu)]
        .map((match) => match[1])
        .filter((asset) => !existsSync(join(publicRoot, asset)))
        .map((asset) => `${page}: /${asset}`);
    });

    expect(missing).toEqual([]);
  });
});

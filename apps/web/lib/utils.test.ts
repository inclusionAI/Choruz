import { describe, expect, it } from "vitest";
import { cn } from "./utils";

describe("cn", () => {
  it("joins truthy class names with a single space", () => {
    expect(cn("a", "b")).toBe("a b");
  });

  it("filters out falsy values", () => {
    expect(cn("a", false && "skip", null, undefined, "b")).toBe("a b");
  });

  it("flattens conditional objects (clsx semantics)", () => {
    expect(cn("base", { active: true, disabled: false })).toBe("base active");
  });

  it("dedupes conflicting Tailwind classes via twMerge (later wins)", () => {
    expect(cn("p-2", "p-4")).toBe("p-4");
    expect(cn("text-red-500", "text-blue-500")).toBe("text-blue-500");
  });

  it("returns empty string for no arguments / all-falsy", () => {
    expect(cn()).toBe("");
    expect(cn(null, undefined, false)).toBe("");
  });
});

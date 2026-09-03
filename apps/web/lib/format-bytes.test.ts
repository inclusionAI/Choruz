import { describe, expect, it } from "vitest";

import { formatFileSize } from "./format-bytes";

describe("formatFileSize", () => {
  it("keeps sub-kilobyte sizes in bytes", () => {
    expect(formatFileSize(0)).toBe("0 B");
    expect(formatFileSize(1023)).toBe("1023 B");
  });

  it("rounds kilobytes to whole numbers", () => {
    expect(formatFileSize(1024)).toBe("1 KB");
    expect(formatFileSize(12_000)).toBe("12 KB");
  });

  it("shows one decimal for megabytes", () => {
    expect(formatFileSize(1024 * 1024)).toBe("1.0 MB");
    expect(formatFileSize(5.25 * 1024 * 1024)).toBe("5.3 MB");
  });
});

import { afterEach, expect, it, vi } from "vitest";
import { nativeFolderPicker } from "./desktop";
import { setActiveTransport } from "./api/transport";

afterEach(() => { setActiveTransport(null); vi.unstubAllGlobals(); });

it("offers native folder selection only for the local desktop transport", () => {
  const chooseFolder = vi.fn();
  vi.stubGlobal("window", { choruzDesktop: { chooseFolder } });
  expect(nativeFolderPicker()).toBe(chooseFolder);
  expect(nativeFolderPicker("remote-device")).toBeUndefined();
  setActiveTransport({ fetch: vi.fn(), socket: vi.fn() });
  expect(nativeFolderPicker()).toBeUndefined();
  setActiveTransport(null);
  vi.stubGlobal("window", {});
  expect(nativeFolderPicker()).toBeUndefined();
});

import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { createHmac, randomBytes } from "node:crypto";
import { expect } from "@playwright/test";
import { test, openRemoteDashboard } from "../fixtures/remote-dashboard";
import { API_BASE, gotoDashboard, signup } from "../fixtures/auth";
import { createCompany, createGroup, sendMessage, uniqueName } from "../fixtures/api";

test("signed pairing registers only authenticated unexpired hosts", async ({ request }) => {
  const gateway = process.env.CHORUZ_REMOTE_CONTROL_GATEWAY_URL!;
  const secret = process.env.CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET!;
  const connects = (query: string) => new Promise<WebSocket | null>((resolve) => {
    const socket = new WebSocket(`${gateway.replace(/^http/, "ws")}/connect?${query}`);
    socket.onopen = () => resolve(socket);
    socket.onerror = () => {};
    socket.onclose = () => resolve(null);
  });
  const close = async (socket: WebSocket | null) => {
    if (!socket || socket.readyState === WebSocket.CLOSED) return;
    const closed = new Promise<void>((resolve) => { socket.onclose = () => resolve(); });
    socket.close(1000);
    await closed;
  };
  for (const kind of ["invalid signature", "expired", "valid", "expires after registration"] as const) {
    const pairing_id = randomBytes(16).toString("base64url");
    const payload = { room: randomBytes(32).toString("hex"), role: "host", scope: "pair", pairing_id,
      exp: Math.floor(Date.now() / 1000) + (kind === "expired" ? -1 : kind === "expires after registration" ? 2 : 60) };
    const encoded = Buffer.from(JSON.stringify(payload)).toString("base64url");
    const signature = createHmac("sha256", kind === "invalid signature" ? `${secret}-wrong` : secret)
      .update(`gateway-ticket\0${encoded}`).digest("hex");
    const host = await connects(`ticket=${encoded}.${signature}`);
    try {
      expect(Boolean(host), kind).toBe(kind === "valid" || kind === "expires after registration");
      if (kind === "expires after registration") {
        await expect.poll(() => Date.now()).toBeGreaterThanOrEqual(payload.exp * 1000);
      }
      const client = await connects(`pairing_id=${pairing_id}&role=pair_client`);
      try { expect(Boolean(client), `${kind} lookup`).toBe(kind === "valid"); }
      finally { await close(client); }
    } finally { await close(host); }
  }
  expect((await request.post(`${gateway}/register-pair`, { data: {} })).status()).toBe(404);
});

test("remote editor saves through the encrypted transport to B", async ({ page, hostedDashboard, ownedResources }) => {
  const root = await mkdtemp(join(homedir(), "choruz-remote-editor-"));
  ownedResources.push(() => rm(root, { recursive: true, force: true }));
  const file = join(root, "owned.txt");
  const { token, principal } = await signup(page, uniqueName("r-edit"), "Owned-password-123!");
  const company = await createCompany(page, token, principal.id, uniqueName("Remote editor"));
  ownedResources.push(async (request) => {
    expect((await request.delete(`${API_BASE}/v1/companies/${company.id}`, { headers: { Authorization: `Bearer ${token}` } })).ok()).toBeTruthy();
  });
  await writeFile(file, "before\n");
  const updated = await page.request.patch(`${API_BASE}/v1/companies/${company.id}`, {
    headers: { Authorization: `Bearer ${token}` },
    data: { actor_id: principal.id, folder_path: root },
  });
  expect(updated.ok()).toBeTruthy();
  const edit = async (value: string, externalChange = false) => {
    await page.locator(".company-selector-btn").click();
    await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
    const explorer = page.locator(".file-tree-section-toggle");
    if (await explorer.getAttribute("aria-expanded") !== "true") await explorer.click();
    const tree = page.getByRole("tree", { name: "File explorer" });
    await expect(tree).toBeVisible();
    await tree.locator(`[data-path="${file}"]`).click();
    const editor = page.locator(".cm-content");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText(value);
    await expect(page.getByRole("button", { name: "Save", exact: true })).toBeEnabled();
    if (externalChange) await writeFile(file, "B agent edit\n");
    await page.getByRole("button", { name: "Save", exact: true }).click();
    if (externalChange) {
      await expect(page.getByRole("button", { name: "Overwrite", exact: true })).toBeVisible();
      await expect(editor).toHaveText(value.trim());
      expect(await readFile(file, "utf8")).toBe("B agent edit\n");
      await page.getByRole("button", { name: "Overwrite", exact: true }).click();
    }
    await expect.poll(() => readFile(file, "utf8")).toBe(value);
  };
  await gotoDashboard(page);
  await edit("saved locally on B\n");
  await openRemoteDashboard(page, token, hostedDashboard.origin);
  await page.reload();
  await expect(page.locator(".chat-shell")).toBeVisible({ timeout: 30_000 });
  await edit("saved remotely on B\n", true);
  expect(hostedDashboard.apiRequests).toEqual([]);
});

test("remote attachment previews and downloads use B's encrypted bytes", async ({ page, hostedDashboard, ownedResources }) => {
  const { token, principal } = await signup(page, uniqueName("r-attach"), "Owned-password-123!");
  const company = await createCompany(page, token, principal.id, uniqueName("Remote attachments"));
  ownedResources.push(async (request) => {
    expect((await request.delete(`${API_BASE}/v1/companies/${company.id}`, { headers: { Authorization: `Bearer ${token}` } })).ok()).toBeTruthy();
  });
  const group = await createGroup(page, token, principal.id, uniqueName("Owned attachments"), [], company.id);
  const emptyGroup = await createGroup(page, token, principal.id, uniqueName("Owned empty"), [], company.id);
  await page.addInitScript((origin) => {
    if (location.origin !== origin) return;
    const ownedBlobAudit = { created: [] as string[], revoked: [] as string[] };
    Object.assign(window, { ownedBlobAudit });
    const create = URL.createObjectURL.bind(URL);
    const revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (blob) => { const url = create(blob); ownedBlobAudit.created.push(url); return url; };
    URL.revokeObjectURL = (url) => { ownedBlobAudit.revoked.push(url); revoke(url); };
  }, hostedDashboard.origin);
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a7WQAAAAASUVORK5CYII=", "base64");
  const payload = Buffer.from("B-only attachment bytes\n");
  const svg = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><script>localStorage.setItem("ownedSvgExecuted","yes")</script></svg>');
  // One black 16×16 VP8 frame, encoded with ffmpeg's libvpx encoder.
  const video = Buffer.from("GkXfo59ChoEBQveBAULygQRC84EIQoKEd2VibUKHgQJChYECGFOAZwEAAAAAAAHpEU2bdLpNu4tTq4QVSalmU6yBoU27i1OrhBZUrmtTrIHYTbuMU6uEElTDZ1OsggElTbuMU6uEHFO7a1OsggHT7AEAAAAAAABZAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAVSalmsirXsYMPQkBNgI1MYXZmNjIuMTIuMTAxV0GNTGF2ZjYyLjEyLjEwMUSJiECPQAAAAAAAFlSua8iuAQAAAAAAAD/XgQFzxYgX1z/y0KRbmZyBACK1nIN1bmSIgQCGhVZfVlA4g4EBI+ODhDuaygDgkLCBELqBEJqBAlWwhFW5gQESVMNn/HNzoGPAgGfImkWjh0VOQ09ERVJEh41MYXZmNjIuMTIuMTAxc3PWY8CLY8WIF9c/8tCkW5lnyKFFo4dFTkNPREVSRIeUTGF2YzYyLjI4LjEwMSBsaWJ2cHhnyKFFo4hEVVJBVElPTkSHkzAwOjAwOjAxLjAwMDAwMDAwMAAfQ7Z1qOeBAKOjgQAAgBACAJ0BKhAAEAAARwiFhYiZhIgCAgAMDWAA/v+rUIAcU7trkbuPs4EAt4r3gQHxggGm8IED", "base64");
  const audio = Buffer.alloc(1_644);
  audio.write("RIFF", 0); audio.writeUInt32LE(audio.length - 8, 4); audio.write("WAVEfmt ", 8);
  audio.writeUInt32LE(16, 16); audio.writeUInt16LE(1, 20); audio.writeUInt16LE(1, 22);
  audio.writeUInt32LE(8_000, 24); audio.writeUInt32LE(16_000, 28);
  audio.writeUInt16LE(2, 32); audio.writeUInt16LE(16, 34); audio.write("data", 36); audio.writeUInt32LE(1_600, 40);
  await gotoDashboard(page);
  await page.locator(".company-selector-btn").click();
  await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
  const groups = page.getByRole("button", { name: /Group Conversations/ });
  if (await groups.getAttribute("aria-expanded") !== "true") await groups.click();
  await page.locator(`[data-conversation-id="${group.id}"]`).click();
  await page.locator(".chat-input-bar input[type='file']").setInputFiles([
    { name: "owned.png", mimeType: "image/png", buffer: png },
    { name: "owned.bin", mimeType: "application/octet-stream", buffer: payload },
    { name: "owned.svg", mimeType: "image/svg+xml", buffer: svg },
    { name: "owned.webm", mimeType: "video/webm", buffer: video },
    { name: "owned.wav", mimeType: "audio/wav", buffer: audio },
  ]);
  await page.locator(".send-btn").click();
  await expect(page.locator("img[alt='owned.png']")).toBeVisible();
  await expect(page.locator(".msg-attachment")).toHaveCount(5);
  await expect(page.locator(".attachment-queue-item")).toHaveCount(0);
  const attachmentPath = await page.locator("img[alt='owned.png']").locator("..").getAttribute("href");
  await sendMessage(page, token, principal.id, group.id,
    `![owned inline](${attachmentPath!.replace("/api/attachments/", "/v1/attachments/")})`);
  await openRemoteDashboard(page, token, hostedDashboard.origin);
  await page.locator(".company-selector-btn").click();
  await page.locator(".company-dropdown-item-name").filter({ hasText: company.name }).click();
  const remoteGroups = page.getByRole("button", { name: /Group Conversations/ });
  if (await remoteGroups.getAttribute("aria-expanded") !== "true") await remoteGroups.click();
  await page.locator(`[data-conversation-id="${group.id}"]`).click();
  const image = page.locator("img[alt='owned.png']");
  await expect.poll(() => image.evaluate((element) => (element as HTMLImageElement).naturalWidth)).toBe(1);
  await expect.poll(() => page.locator("img[alt='owned inline']").evaluate((element) => (element as HTMLImageElement).naturalWidth)).toBe(1);
  await expect.poll(() => page.locator("video").evaluate((element) => element.videoWidth)).toBe(16);
  await expect.poll(() => page.locator("audio").evaluate((element) => element.duration)).toBeCloseTo(0.1);
  for (const [selector, bytes] of [["video", video], ["audio", audio]] as const) {
    const received = await page.locator(selector).evaluate(async (element) => [...new Uint8Array(await (await fetch((element as HTMLMediaElement).src)).arrayBuffer())]);
    expect(Buffer.from(received)).toEqual(bytes);
  }
  const svgDownloadEvent = page.waitForEvent("download", { timeout: 5_000 });
  await page.locator("img[alt='owned.svg']").locator("..").click();
  const svgDownload = await svgDownloadEvent;
  expect(svgDownload.suggestedFilename()).toBe("owned.svg");
  expect(await readFile((await svgDownload.path())!)).toEqual(svg);
  expect(await page.evaluate(() => localStorage.getItem("ownedSvgExecuted"))).toBeNull();
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("link").filter({ hasText: "owned.bin" }).click();
  const download = await downloadEvent;
  expect(download.suggestedFilename()).toBe("owned.bin");
  const saved = await download.path();
  expect(saved).not.toBeNull();
  expect(await readFile(saved!)).toEqual(payload);
  await page.locator(`[data-conversation-id="${emptyGroup.id}"]`).click();
  await expect.poll(() => page.evaluate(() => {
    const audit = (window as unknown as { ownedBlobAudit: { created: string[]; revoked: string[] } }).ownedBlobAudit;
    return { created: audit.created.length, pending: audit.created.filter((url) => !audit.revoked.includes(url)).length };
  })).toEqual({ created: 6, pending: 0 });
  expect(hostedDashboard.apiRequests).toEqual([]);
});

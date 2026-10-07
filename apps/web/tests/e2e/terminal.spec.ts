import { expect, test, request } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, writeFile, readFile, realpath, mkdtemp, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import { postgresQueryClient } from "../../lib/groups/group-provisioning-db";
import { API_BASE, WEB_BASE, login, gotoDashboard } from "../fixtures/auth";
import { createGroup, createCompany, deleteCompany, provisionAgent, sendMessage, uniqueName } from "../fixtures/api";
import { runtimeTest } from "../fixtures/runtime-device";

test("new task uses the project's shared learning through its native CLI", async ({ page }) => {
  test.setTimeout(90_000);
  const { token, principal } = await login(page);
  const company = await createCompany(page, token, principal.id, uniqueName("shared-learning"));
  const directory = await mkdtemp(path.join(homedir(), "choruz-shared-learning-"));
  const headers = { Authorization: `Bearer ${token}` };
  const bindings: string[] = [];
  try {
    const updated = await page.request.patch(`${API_BASE}/v1/companies/${company.id}`, { headers, data: { actor_id: principal.id, folder_path: directory } });
    expect(updated.ok()).toBe(true);
    const native = path.join(directory, "native.py");
    const analyst = path.join(directory, "analyst.py");
    const binary = path.join(directory, "codex");
    await writeFile(native, await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/structured-cli.py")), { mode: 0o700 });
    await writeFile(analyst, await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/experience-analyst.py")), { mode: 0o700 });
    await writeFile(binary, `#!/usr/bin/env python3\nimport os,sys\np=${JSON.stringify(analyst)} if sys.argv[1:2]==['exec'] else ${JSON.stringify(native)}\nos.execv(sys.executable,[sys.executable,p,*sys.argv[1:]])\n`, { mode: 0o700 });
    const agents = [];
    for (const role of ["source", "analyst"]) {
      const response = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: {
        name: uniqueName(`shared-${role}`), driver_type: "codex_terminal", instructions: "Shared task acceptance.", workspace_id: company.id, workspace_path: directory,
      } });
      expect(response.status(), await response.text()).toBe(201);
      const agent = await response.json();
      bindings.push(agent.binding.id);
      agents.push(agent);
    }
    const [source, reviewer] = agents;
    const db = await postgresQueryClient();
    await db.query("UPDATE harness_account SET models_json=$1::jsonb WHERE company_id=$2 AND id IN (SELECT config_json->>'harness_account_id' FROM agent_runtime_bindings WHERE id=ANY($3::text[]))", [JSON.stringify([{ id: "fixture-model", name: "Fixture model" }]), company.id, bindings]);
    await db.query("UPDATE agent_runtime_bindings SET config_json=config_json||$1::jsonb WHERE id=ANY($2::text[])", [JSON.stringify({ binary_path: binary, model: "fixture-model" }), bindings]);
    const configured = await page.request.put(`${API_BASE}/v1/runtime/bindings/${source.binding.id}/experience`, { headers, data: {
      enabled: true, analyst_binding_id: reviewer.binding.id, reuse_for_new_tasks: true,
    } });
    expect(configured.ok()).toBe(true);
    const revision = randomUUID();
    // Seed a reviewed learning result, not provisioning, policy lookup or send.
    // The external CLI fixture replaces only provider execution.
    await db.query("UPDATE experience_policy SET next_check_at=NOW()+INTERVAL '1 day',lease_token=NULL,lease_until=NULL WHERE binding_id=$1", [source.binding.id]);
    await db.query("INSERT INTO experience_revision(id,binding_id,workspace_id,policy_generation,source_digest,source_references,analysis,instruction,disposition,validation) SELECT $1,binding_id,workspace_id,generation,$1,'[]','Reviewed outcome','Verify before reporting completion.','active',$3::jsonb FROM experience_policy WHERE binding_id=$2", [revision, source.binding.id, JSON.stringify({ review: "passed", team: { review: "passed", config: { order: "parallel", members: [{ name: "reviewer", prompt: "Check the result." }] } } })]);
    await db.query("UPDATE experience_policy SET active_revision_id=$2 WHERE binding_id=$1", [source.binding.id, revision]);
    await page.route("**/api/drivers/availability", (route) => route.fulfill({ json: { drivers: [{ driverId: "codex_terminal", label: "Codex", status: "available", available: true }] } }));
    let child: { binding: { id: string; model?: string } } | undefined;
    await page.route("**/api/agents/provision", async (route) => {
      expect(route.request().postDataJSON().inherit_learning).toBe(true);
      const response = await route.fetch();
      expect(response.status()).toBe(201);
      child = await response.json();
      bindings.push(child!.binding.id);
      await db.query("UPDATE agent_runtime_bindings SET config_json=config_json||$1::jsonb WHERE id=$2", [JSON.stringify({ binary_path: binary }), child!.binding.id]);
      await route.fulfill({ response });
    });
    await gotoDashboard(page);
    await page.getByRole("button", { name: "Select company" }).click();
    await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
    await page.getByRole("button", { name: "New task", exact: true }).click();
    await page.getByLabel("Task instructions").fill("Inspect the workspace");
    await page.getByRole("button", { name: "Start task", exact: true }).click();
    const session = page.getByRole("region", { name: "Agent session" });
    await expect(session.getByRole("button", { name: "Allow once" })).toBeVisible({ timeout: 30_000 });
    await session.getByRole("button", { name: "Allow once" }).click();
    await expect(session.getByRole("status")).toHaveText("ready");
    expect(child!.binding.model).toBe("fixture-model");
    const shared = await page.request.get(`${API_BASE}/v1/runtime/bindings/${child!.binding.id}/experience`, { headers });
    expect((await shared.json()).policy).toMatchObject({ binding_id: source.binding.id, active_revision_id: revision, reuse_for_new_tasks: true });
    const received = JSON.stringify(JSON.parse(await readFile(path.join(directory, ".fixture-native.json"), "utf8")));
    expect(received).toContain(`[choruz-experience revision=${revision}]`);
    expect(received).toContain("Verify before reporting completion.");
    expect(received).toContain(`[choruz-team revision=${revision}]`);
    expect(received).toContain("Inspect the workspace");
    expect((await db.query("SELECT COUNT(*)::int AS copies FROM experience_policy WHERE binding_id=$1", [child!.binding.id])).rows[0].copies).toBe(0);
  } finally {
    for (const id of bindings) await page.request.post(`${API_BASE}/v1/runtime/bindings/${id}/session/commands`, { headers, data: { action: "close" } }).catch(() => {});
    await deleteCompany(page, token, company.id);
    await rm(directory, { recursive: true, force: true });
  }
});

test("workbench hides collaboration and starts a real task through the existing runtime", async ({ page }, testInfo) => {
  test.setTimeout(90_000);
  const { token, principal } = await login(page);
  const company = await createCompany(page, token, principal.id, uniqueName("workbench"));
  const directory = await mkdtemp(path.join(tmpdir(), "choruz-workbench-"));
  try {
    const group = await createGroup(page, token, principal.id, uniqueName("background"), [], company.id);
    const binary = path.join(directory, "codex");
    await writeFile(binary, await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/structured-cli.py")), { mode: 0o700 });
    await page.route("**/api/drivers/availability", (route) => route.fulfill({ json: { drivers: [{ driverId: "codex_terminal", label: "Codex", status: "available", available: true }] } }));
    let created: { binding: { id: string }; workspace_path: string; conversation: { id: string } } | undefined;
    let creates = 0;
    let allowReady = false;
    await page.route(/\/api\/v1\/runtime\/bindings\/[^/]+\/session(?:\?|$)/, async (route) => {
      const response = await route.fetch();
      const snapshot = await response.json();
      await route.fulfill({ response, json: allowReady ? snapshot : { ...snapshot, status: "starting" } });
    });
    await page.route("**/api/v1/bootstrap*", (route) => creates > 0
      ? route.fulfill({ status: 503, json: { error: "temporary bootstrap failure" } })
      : route.continue());
    await page.route("**/api/agents/provision", async (route) => {
      creates++;
      const response = await route.fetch();
      expect(response.status()).toBe(201);
      created = await response.json();
      const db = await postgresQueryClient();
      await db.query("UPDATE agent_runtime_bindings SET config_json = (config_json - 'model') || $1::jsonb WHERE id=$2", [JSON.stringify({ binary_path: binary }), created!.binding.id]);
      if (creates === 1) { await route.abort("failed"); return; }
      await route.fulfill({ response });
    });
    await page.goto(`${WEB_BASE}/dashboard`);
    await page.evaluate(({ companyId, groupId }) => {
      localStorage.setItem("choruz_active_conv", groupId);
      localStorage.setItem(`choruz_active_company:${companyId.principal}`, companyId.id);
    }, { companyId: { id: company.id, principal: principal.id }, groupId: group.id });
    await page.reload();
    await expect(page.getByRole("heading", { name: "What would you like to work on?" })).toBeVisible();
    await expect(page.locator(`[data-conversation-id="${group.id}"]`)).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("workbench-home.png") });
    await expect(page.getByRole("button", { name: "Background collaboration", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "Actions menu", exact: true }).click();
    await page.getByRole("button", { name: "Background collaboration", exact: true }).click();
    const section = page.getByRole("group", { name: "Group Conversations", exact: true });
    const expand = section.getByRole("button", { name: "Group Conversations", exact: true });
    if (await expand.getAttribute("aria-expanded") !== "true") await expand.click();
    await expect(page.locator(`[data-conversation-id="${group.id}"]`)).toBeVisible();
    await page.locator(`[data-conversation-id="${group.id}"]`).click();
    await page.getByRole("button", { name: "Actions menu", exact: true }).click();
    await page.getByRole("button", { name: "Hide background collaboration", exact: true }).click();
    const refreshed = page.waitForResponse((response) => response.url().includes("/api/v1/bootstrap") && response.status() === 200);
    await createGroup(page, token, principal.id, uniqueName("new-background"), [], company.id);
    await (await refreshed).finished();
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(page.getByRole("group", { name: "Group Conversations", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "New task", exact: true }).click();
    await page.getByLabel("Task instructions").fill("Inspect the workspace");
    await page.getByRole("button", { name: "Start task", exact: true }).click();
    await expect(page.getByRole("alert").filter({ hasText: /fetch|network/i })).toBeVisible();
    const firstBinding = created!.binding.id;
    await expect(page.getByLabel("Task instructions")).toHaveValue("Inspect the workspace");
    await page.getByRole("button", { name: "Start task", exact: true }).click();
    const session = page.getByRole("region", { name: "Agent session" });
    await expect(session.getByLabel("Message Agent")).toHaveValue("Inspect the workspace");
    await session.getByLabel("Message Agent").fill("Inspect the workspace with my revised instructions");
    await page.unroute("**/api/v1/bootstrap*");
    await page.reload();
    await expect(session.getByLabel("Message Agent")).toHaveValue("Inspect the workspace with my revised instructions");
    allowReady = true;
    await expect(session.getByRole("button", { name: "Allow once" })).toBeVisible({ timeout: 30_000 });
    await expect(session.locator(".agent-session-item.is-user")).toContainText("Inspect the workspace with my revised instructions");
    await page.unroute("**/api/v1/bootstrap*");
    expect(created!.binding.id).toBe(firstBinding);
    await session.getByRole("button", { name: "Allow once" }).click();
    await expect(session.getByText("Verified workspace on selected device", { exact: true })).toBeVisible();
    expect(creates).toBe(2);
    expect(await readFile(path.join(created!.workspace_path, "approved-on-device"), "utf8")).toBe(created!.workspace_path);
    await page.getByRole("button", { name: "New task", exact: true }).click();
    await page.locator(`[data-conversation-id="${created!.conversation.id}"]`).click();
    await expect(session.getByText("Verified workspace on selected device", { exact: true })).toHaveCount(1);
    await page.screenshot({ path: testInfo.outputPath("workbench-task.png") });
    await page.getByRole("button", { name: "New task", exact: true }).click();
    await page.getByLabel("Task instructions").fill("Keep this draft");
    await page.reload();
    await expect(page.getByLabel("Task instructions")).toHaveValue("Keep this draft");
    await page.setViewportSize({ width: 390, height: 844 });
    await expect.poll(() => page.locator(".chat-sidebar").evaluate(element => element.getBoundingClientRect().right)).toBeLessThanOrEqual(0);
    await expect(page.getByRole("button", { name: "Start task", exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath("workbench-mobile.png") });
  } finally {
    try {
      if (!page.isClosed()) await page.unrouteAll({ behavior: "wait" });
      await deleteCompany(page, token, company.id);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  }
});

test("account removal remains pending while its device is offline",async({page})=>{
  const {token,principal}=await login(page);
  const company=await createCompany(page,token,principal.id,uniqueName("offline-removal"));
  const headers={Authorization:`Bearer ${token}`};
  try{
    const pairing=await page.request.post(`${API_BASE}/v1/companies/${company.id}/runtime-host-pairings`,{headers});
    expect(pairing.ok(),await pairing.text()).toBe(true);
    const {code}=await pairing.json();
    const joined=await page.request.post(`${API_BASE}/v1/runtime-host-pairings/redeem`,{data:{code,name:"Offline device"}});
    expect(joined.ok(),await joined.text()).toBe(true);
    const {host}=await joined.json();
    const account=randomUUID();
    const db=await postgresQueryClient();
    await db.query("INSERT INTO harness_account(id,company_id,runtime_host_id,driver_type,name,profile_kind,status) VALUES($1,$2,$3,'codex_terminal','offline-default','default','active')",[account,company.id,host.id]);
    const removed=await page.request.delete(`${WEB_BASE}/api/harness-accounts/${account}?company_id=${company.id}`);
    expect(removed.status(),await removed.text()).toBe(202);
    expect((await removed.json()).removal_pending).toBe(true);
    const row=(await db.query("SELECT disabled_at,removal_requested_at,removal_completed_at FROM harness_account WHERE id=$1",[account])).rows[0];
    expect(row.disabled_at).toBeTruthy();
    expect(row.removal_requested_at).toBeTruthy();
    expect(row.removal_completed_at).toBeNull();
    const visible=await page.request.get(`${WEB_BASE}/api/harness-accounts?company_id=${company.id}&runtime_host_id=${host.id}`);
    expect((await visible.json()).accounts.find((item:{id:string})=>item.id===account).removalPending).toBe(true);
  }finally{await deleteCompany(page,token,company.id);}
});

runtimeTest("removing a remote account stops its native process and preserves another account", async ({ page,device }) => {
  test.setTimeout(90_000);
  const {home,company,host,headers}=device;
  const original=path.join(home,"structured-cli.py");
  const binary=path.join(home,"account-cli");
  await writeFile(original,await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/structured-cli.py")),{mode:0o700});
  const wrapper=`#!/bin/sh\nif [ "$1" = "exec" ]; then\n  printf '%s' "$$" > "$PWD/headless.pid"\n  sleep 120 &\n  printf '%s' "$!" > "$PWD/descendant.pid"\n  wait\nelse\n  printf '%s' "$$" > "$PWD/native.pid"\n  exec "${original}" "$@"\nfi\n`;
  await writeFile(binary,wrapper,{mode:0o700});
  await writeFile(path.join(home,"bin","codex"),wrapper,{mode:0o700});
  const alive=(pid:number)=>{try{process.kill(pid,0);return true;}catch(error){if((error as NodeJS.ErrnoException).code==="ESRCH")return false;throw error;}};
  const db=await postgresQueryClient();
  const agents:{account:string;binding:string;workspace:string;pid:number;agent:string;name:string}[]=[];
  const extraPids:number[]=[];
  try{
    for(const kind of ["default","isolated"]){
      const account=randomUUID();
      const profile=kind==="default"?path.join(home,".codex"):path.join(home,"accounts",account,"codex");
      await mkdir(profile,{recursive:true});
      await writeFile(path.join(profile,"auth.json"),JSON.stringify({fixture:kind}));
      await db.query("INSERT INTO harness_account(id,company_id,runtime_host_id,driver_type,name,profile_kind,status) VALUES($1,$2,$3,'codex_terminal',$4,$4,'active')",[account,company.id,host.id,kind]);
      const response=await page.request.post(`${WEB_BASE}/api/agents/provision`,{data:{name:uniqueName(`retire-${kind}`),instructions:"Account lifecycle acceptance.",driver_type:"codex_terminal",workspace_id:company.id,runtime_host_id:host.id,harness_account_id:account}});
      expect(response.status(),await response.text()).toBe(201);
      const created=await response.json();
      await db.query("UPDATE agent_runtime_bindings SET config_json=(config_json - 'model') || $2::jsonb WHERE id=$1",[created.binding.id,JSON.stringify({binary_path:binary})]);
      const started=await page.request.post(`${API_BASE}/v1/runtime/bindings/${created.binding.id}/session`,{headers});
      expect(started.ok(),await started.text()).toBe(true);
      await expect.poll(()=>readFile(path.join(created.workspace_path,"native.pid"),"utf8").catch(()=>"")).toMatch(/^\d+$/);
      const pid=Number(await readFile(path.join(created.workspace_path,"native.pid"),"utf8"));
      agents.push({account,binding:created.binding.id,workspace:created.workspace_path,pid,agent:created.agent.id,name:created.agent.name});
      await expect.poll(async()=>{const state=await page.request.get(`${API_BASE}/v1/runtime/bindings/${created.binding.id}/session`,{headers});return state.ok()?(await state.json()).status:null;}).toBe("ready");
    }
    const [removed,retained]=agents;
    expect(alive(removed.pid)).toBe(true);
    expect(alive(retained.pid)).toBe(true);
    const group=await createGroup(page,device.session.token,device.session.principal.id,uniqueName("account-headless"),[removed.agent],company.id);
    await sendMessage(page,device.session.token,device.session.principal.id,group.id,`@${removed.name} Run a long task.`);
    for(const file of ["headless.pid","descendant.pid"]){
      await expect.poll(()=>readFile(path.join(removed.workspace,file),"utf8").catch(()=>""),{timeout:15000}).toMatch(/^\d+$/);
      extraPids.push(Number(await readFile(path.join(removed.workspace,file),"utf8")));
    }
    const response=await page.request.delete(`${WEB_BASE}/api/harness-accounts/${removed.account}?company_id=${company.id}`);
    expect(response.ok(),await response.text()).toBe(true);
    await expect.poll(()=>alive(removed.pid)).toBe(false);
    for(const pid of extraPids)await expect.poll(()=>alive(pid)).toBe(false);
    expect(alive(retained.pid)).toBe(true);
    await expect.poll(async()=> (await db.query("SELECT removal_completed_at FROM harness_account WHERE id=$1",[removed.account])).rows[0].removal_completed_at).toBeTruthy();
    const rejected=await page.request.post(`${API_BASE}/v1/runtime/bindings/${removed.binding}/session`,{headers});
    expect(rejected.status()).toBe(403);
    const other=await page.request.get(`${API_BASE}/v1/runtime/bindings/${retained.binding}/session`,{headers});
    expect(other.ok(),await other.text()).toBe(true);
    expect((await other.json()).status).toBe("ready");
    expect(JSON.parse(await readFile(path.join(home,".codex","auth.json"),"utf8"))).toEqual({fixture:"default"});
  } finally {
    for(const pid of extraPids){if(alive(pid))process.kill(pid,"SIGKILL");await expect.poll(()=>alive(pid)).toBe(false);}
    for(const agent of agents){
      if(alive(agent.pid))process.kill(agent.pid,"SIGKILL");
      await expect.poll(()=>alive(agent.pid)).toBe(false);
    }
  }
});

runtimeTest("background experience follows the selected remote Agent and applies only to later turns", async ({ page, device }) => {
  test.setTimeout(180_000);
  const { company, host, headers, home } = device;
  const cli = path.join(home, "learning-source-cli");
  const analystCli = path.join(home, "learning-analyst-cli");
  await writeFile(cli, await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/structured-cli.py")), { mode: 0o700 });
  await writeFile(analystCli, await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/experience-analyst.py")), { mode: 0o700 });
  const agents = [];
  for (const [role, driver, binary] of [["source", "claude_terminal", cli], ["analyst", "codex_terminal", analystCli]]) {
    const response = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: {
      name: uniqueName(`learning-${role}`), instructions: "Experience learning acceptance.", driver_type: driver,
      workspace_id: company.id, ...(role === "source" ? { runtime_host_id: host.id } : {}),
    } });
    expect(response.status(), await response.text()).toBe(201);
    const agent = await response.json();
    const db = await postgresQueryClient();
    await db.query("UPDATE harness_account SET models_json=$1::jsonb WHERE company_id=$2 AND id=(SELECT config_json->>'harness_account_id' FROM agent_runtime_bindings WHERE id=$3)", [JSON.stringify([{ id: "learning-fixture", name: "Learning fixture" }]), company.id, agent.binding.id]);
    await db.query("UPDATE agent_runtime_bindings SET config_json=(config_json-'model') || $1::jsonb WHERE id=$2", [JSON.stringify({ binary_path: binary }),agent.binding.id]);
    agents.push(agent);
  }
  const [target, analyst] = agents;
  const exec = promisify(execFile);
  const root = path.resolve("../..");
  await exec("cargo", ["build", "-p", "choruz-cli"], { cwd: root, timeout: 120_000 });
  const settingsFile = path.join(home, "learning-settings.json");
  await writeFile(settingsFile, JSON.stringify({ enabled: false, analyst_binding_id: analyst.binding.id }));
  const runCli = async (...args: string[]) => JSON.parse((await exec(path.join(root, "target/debug/choruz"), args, {
    timeout: 30_000, env: { ...process.env, CHORUZ_API_BASE_URL: API_BASE, CHORUZ_SESSION_TOKEN: headers.Authorization.replace("Bearer ", "") },
  })).stdout);
  await runCli("learning", "configure", target.binding.id, settingsFile);
  const configured = await runCli("learning", "show", target.binding.id);
  expect(configured.policy).toMatchObject({ enabled: false, analyst_binding_id: analyst.binding.id });
  const persisted = await page.request.get(`${API_BASE}/v1/runtime/bindings/${target.binding.id}/experience`, { headers });
  expect(persisted.ok()).toBe(true);
  expect((await persisted.json()).policy).toEqual(configured.policy);
  await gotoDashboard(page);
  await page.getByRole("button", { name: "Select company" }).click();
  await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
  await page.locator(".conv-item").filter({ hasText: target.agent.name }).first().click();
  const session = page.getByRole("region", { name: "Agent session" });
  const send = async (text: string) => {
    await expect(session.getByRole("status")).toHaveText("ready");
    await session.getByLabel("Message Agent").fill(text);
    await session.getByRole("button", { name: "Send", exact: true }).click();
    await session.getByRole("button", { name: "Allow once" }).click();
    await expect(session.getByRole("status")).toHaveText("ready");
  };
  await send("Your completion claim omitted the required check. Verify the workspace before reporting completion.");
  await session.getByRole("button", { name: "Experience learning", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Experience learning", exact: true });
  await dialog.getByLabel("Analysis Agent").selectOption(analyst.binding.id);
  await dialog.getByLabel("Enable background learning").check();
  await dialog.getByLabel("Evaluate and optimize against a fixed suite").check();
  await expect(dialog.getByLabel("Task model", { exact: true })).toHaveValue("");
  await expect(dialog.getByLabel("Analysis model", { exact: true })).toHaveValue("");
  await dialog.getByLabel("Task model", { exact: true }).fill("learning-fixture");
  await dialog.getByLabel("Analysis model", { exact: true }).fill("learning-fixture");
  await dialog.getByLabel("Suite name").fill("Verification format");
  for (let i = 0; i < 3; i++) {
    await dialog.getByLabel("Task input", { exact: true }).nth(i).fill(`Check independent example ${i}`);
    await dialog.getByLabel("Expected answer", { exact: true }).nth(i).fill("CHECKED");
    await dialog.getByLabel("Assessment", { exact: true }).nth(i).selectOption("judge");
    await dialog.getByLabel("Acceptance criteria", { exact: true }).nth(i).fill("The answer must report the verified check result.");
  }
  await dialog.getByText("Search budget and ordering", { exact: true }).click();
  await dialog.getByLabel("Maximum task evaluations", { exact: true }).fill("4");
  await dialog.getByLabel("Automatically apply a measured and reviewed improvement").check();
  await dialog.getByRole("button", { name: "Save settings", exact: true }).click();
  await expect(dialog.getByRole("button", { name: "Save settings", exact: true })).toBeEnabled();
  const community = dialog.getByRole("region", { name: "Behavior community" });
  await expect(community.getByRole("checkbox", { name: "Search accepted community experience" })).not.toBeChecked();
  await expect(community.getByRole("checkbox", { name: "Automatically trial applicable solutions through learning review" })).toBeDisabled();
  await expect(community.getByRole("checkbox", { name: "Contribute independently reviewed, redacted experience publicly" })).not.toBeChecked();
  await community.getByRole("button", { name: "Save community permissions" }).click();
  await expect(community.getByRole("button", { name: "Save community permissions" })).toBeEnabled();
  const communityResponse = await page.request.get(`${API_BASE}/v1/runtime/bindings/${target.binding.id}/experience/community`, { headers });
  expect(communityResponse.ok()).toBeTruthy();
  expect((await communityResponse.json()).settings).toEqual({ search: false, automatic_trial: false, contribute: false });
  await page.screenshot({ path: test.info().outputPath("behavior-permissions.png"), animations: "disabled" });
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  const endpoint = `${API_BASE}/v1/runtime/bindings/${target.binding.id}/experience`;
  const status = async () => {
    const response = await page.request.get(endpoint, { headers });
    expect(response.ok(), await response.text()).toBe(true);
    return response.json();
  };
  await expect.poll(async () => (await status()).policy?.active_revision_id, { timeout: 45_000 }).toBeTruthy();
  const learned = await status();
  expect(learned.policy.optimization_settings.suite.name).toBe("Verification format");
  expect(learned.policy.optimization_settings.suite.cases[0].check).toEqual({ type: "judge", expected: "CHECKED", rubric: "The answer must report the verified check result." });
  const evaluations = await page.request.get(`${endpoint}/evaluations`, { headers });
  const evaluated = (await evaluations.json()).evaluations[0];
  expect(evaluated.application_status).toBe("applied");
  expect(evaluated.search_summary.scores).toEqual([0, 1, 0, 1]);
  let revision = learned.policy.active_revision_id;
  expect(learned.revisions[0].validation.research).toContain("observable check evidence");
  expect(learned.revisions[0].source_references[0]).toMatch(/^[a-zA-Z0-9-]+:\d+$/);
  await send("Compare the next approaches.");
  await expect(session.getByText(`Choruz learned context · ${revision.slice(0,8)}`, { exact: true })).toHaveCount(1);
  const native = JSON.parse(await readFile(path.join(target.workspace_path, ".fixture-native.json"), "utf8"));
  const userInputs = native.claude.filter((row: { type: string; message: { content: { type: string }[] } }) => row.type === "user" && row.message.content[0].type === "text");
  expect(userInputs[0].message.content[0].text).not.toContain("choruz-experience");
  expect(userInputs[1].message.content[0].text).toContain(`revision=${revision}`);
  expect(userInputs[1].message.content[0].text).toContain("Verify required checks before reporting completion.");
  await send("For that separate task, your completion claim again missed the required check.");
  // A second fixed suite distinguishes actual collaborator execution from the
  // prompt-only improvement. Only external model generation is replaced.
  await session.getByRole("button", { name: "Experience learning", exact: true }).click();
  await expect(dialog.getByRole("region", { name: "Evaluation history" })).toContainText("Application: applied");
  const localCard = community.locator("details").filter({ hasText: "Completion without verification" }).first();
  await expect(localCard.locator("summary")).toBeVisible({ timeout: 30_000 });
  await localCard.locator("summary").click();
  await expect(localCard).toContainText("Execution model:");
  await expect(localCard).toContainText("The Agent claimed completion without checking.");
  await page.screenshot({ path: test.info().outputPath("behavior-community.png"), animations: "disabled" });
  await expect(dialog.getByRole("table")).toContainText("Selected winner");
  for (let i = 0; i < 3; i++) {
    await dialog.getByLabel("Expected answer", { exact: true }).nth(i).fill("TEAM_CHECKED");
  }
  await dialog.getByLabel("Evolve the internal execution team").check();
  await dialog.getByLabel("Maximum total agents per task").fill("3");
  await dialog.getByText("Search budget and ordering", { exact: true }).click();
  await dialog.getByLabel("Maximum task evaluations", { exact: true }).fill("24");
  await dialog.getByLabel("Maximum proposals", { exact: true }).fill("2");
  await dialog.getByRole("button", { name: "Save settings", exact: true }).click();
  await expect(dialog.getByRole("button", { name: "Save settings", exact: true })).toBeEnabled();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  const schedulingDb = await postgresQueryClient();
  await expect.poll(async () => {
    await schedulingDb.query("UPDATE experience_policy SET next_check_at=NOW() WHERE binding_id=$1 AND lease_token IS NULL", [target.binding.id]);
    const result = await status();
    return result.revisions.find((row: { id: string }) => row.id === result.policy.active_revision_id)?.validation.team?.review;
  }, { timeout: 75_000 }).toBe("passed");
  revision = (await status()).policy.active_revision_id;
  const selected = (await status()).revisions.find((row: { id: string }) => row.id === revision);
  expect(selected.validation.team.config.members.map((member: { name: string }) => member.name)).toEqual(["derive", "check"]);
  const turnFile = path.join(home, "native-turn.json");
  await writeFile(turnFile, JSON.stringify({ input: "Inspect the next native workspace change." }));
  const prepared = await runCli("learning", "prepare", target.binding.id, turnFile);
  expect(prepared.revision_id).toBe(revision);
  expect(prepared.prompt).toContain(`[choruz-team revision=${revision}]`);
  expect(prepared.prompt).toContain("Plan task-specific observable checks");
  // Replace only the native model process, not CLI authentication, host dispatch
  // or revision selection. The external consumer receives the prepared input.
  await exec(cli, ["--print", "--dangerously-skip-permissions", prepared.prompt], { cwd: target.workspace_path });
  expect(JSON.parse(await readFile(path.join(target.workspace_path, "headless-review-input.json"), "utf8")).prompt).toBe(prepared.prompt);
  await send("Inspect the next workspace change.");
  await expect(session.getByText(`Independent check plan · ${revision.slice(0,8)}`, { exact: true })).toHaveCount(0);
  await expect(session.getByText(`[choruz-team revision=${revision}]`, { exact: false })).toBeVisible();
  const reviewedNative = JSON.parse(await readFile(path.join(target.workspace_path, ".fixture-native.json"), "utf8"));
  expect(JSON.stringify(reviewedNative)).toContain(`[choruz-team revision=${revision}]`);
  expect(JSON.stringify(reviewedNative)).toContain("Plan task-specific observable checks for the task.");
  await session.getByLabel("Message Agent").fill("wait");
  const cancelledSend = page.waitForResponse((response) =>
    response.url().endsWith(`/runtime/bindings/${target.binding.id}/session/commands`)
    && response.request().postDataJSON().action === "send");
  await session.getByRole("button", { name: "Send", exact: true }).click();
  await expect(session.getByRole("status")).toHaveText("running");
  await expect(session.getByRole("button", { name: "Stop", exact: true })).toBeEnabled();
  await session.getByRole("button", { name: "Stop", exact: true }).click();
  expect((await cancelledSend).status()).toBe(409);
  await expect(session.getByRole("status")).toHaveText("ready");
  expect(JSON.parse(await readFile(path.join(target.workspace_path, ".fixture-native.json"), "utf8"))).toEqual(reviewedNative);
  await writeFile(path.join(home, "bin", "claude"), await readFile(cli), { mode: 0o700 });
  const group = await createGroup(page, device.session.token, device.session.principal.id, uniqueName("reviewed-group"), [target.agent.id], company.id);
  await sendMessage(page, device.session.token, device.session.principal.id, group.id, `@${target.agent.name} Inspect the next group task.`);
  await expect.poll(async () => readFile(path.join(target.workspace_path, "headless-review-input.json"), "utf8").catch(() => ""), { timeout: 30_000 }).toContain(`[choruz-team revision=${revision}]`);
  await session.getByRole("button", { name: "Experience learning", exact: true }).click();
  await dialog.getByLabel("Enable background learning").uncheck();
  await dialog.getByRole("button", { name: "Refresh history", exact: true }).click();
  await expect(dialog.getByLabel("Enable background learning")).not.toBeChecked();
  await dialog.getByRole("button", { name: "Save settings", exact: true }).click();
  await expect.poll(async () => (await status()).policy.enabled).toBe(false);
  const disabledPreparation = await runCli("learning", "prepare", target.binding.id, turnFile);
  expect(disabledPreparation).toEqual({ prompt: "Inspect the next native workspace change.", revision_id: null });
  const decisions = dialog.getByRole("region", { name: "Fast decisions", exact: true });
  await expect(decisions.getByRole("checkbox", { name: "Allow this Agent’s task evidence to be sent to TypeSafe" })).not.toBeChecked();
  await decisions.getByRole("checkbox", { name: "Allow this Agent’s task evidence to be sent to TypeSafe" }).check();
  await decisions.getByLabel("Jev model", { exact: true }).fill("test-decision-model");
  await decisions.getByLabel("Program-building Agent").selectOption(analyst.binding.id);
  const confidence = decisions.getByLabel("Minimum routing confidence", { exact: true });
  const saveDecisions = decisions.getByRole("button", { name: "Save decision settings", exact: true });
  await confidence.fill("");
  await expect(saveDecisions).toBeDisabled();
  await confidence.fill("1.1");
  await expect(saveDecisions).toBeDisabled();
  await confidence.fill("0");
  await expect(saveDecisions).toBeEnabled();
  await confidence.fill("0.9");
  await decisions.getByRole("button", { name: "Save decision settings", exact: true }).click();
  await expect(decisions.getByRole("status")).toHaveText("Decision settings saved.");
  await expect.poll(async () => (await status()).policy.decision_settings).toEqual({ model: "test-decision-model", minimum_confidence: 0.9, classify: true, supervise: true, assist_turns: false, complete_turns: false, builder_binding_id: analyst.binding.id });
  // Keep provider calls out of this UI contract; learning remains disabled.
  await decisions.getByLabel("Assist later Agent turns with the selected program").check();
  await saveDecisions.click();
  await expect.poll(async () => (await status()).policy.decision_settings.assist_turns).toBe(true);
  await decisions.getByLabel("Automatically complete applicable turns with the evaluated program").check();
  await saveDecisions.click();
  await expect.poll(async () => (await status()).policy.decision_settings.complete_turns).toBe(true);
  await decisions.getByRole("checkbox", { name: "Allow this Agent’s task evidence to be sent to TypeSafe" }).uncheck();
  await decisions.getByRole("button", { name: "Save decision settings", exact: true }).click();
  await expect.poll(async () => (await status()).policy.decision_settings).toBeNull();
  await schedulingDb.query("UPDATE experience_policy SET enabled=TRUE,next_check_at=NOW()+INTERVAL '1 day',decision_settings=$2 WHERE binding_id=$1", [target.binding.id, JSON.stringify({ model: "test-decision-model", minimum_confidence: 0.9, classify: true, supervise: true, assist_turns: true, builder_binding_id: analyst.binding.id })]);
  const automation = dialog.getByRole("region", { name: "Automatic browser work" });
  await automation.getByLabel("Enable automatic learning and reuse").check();
  await automation.getByLabel("Browser on this device").fill("fixture-browser");
  await automation.getByLabel("Permitted page URLs, one per line").fill("https://example.org/editor");
  await automation.getByLabel("Permitted tasks").fill("Save work drafts");
  await automation.getByRole("button", { name: "Save browser automation" }).click();
  await expect(automation.getByRole("status")).toHaveText("Automation enabled");
  await expect.poll(async () => (await schedulingDb.query("SELECT settings FROM browser_automation WHERE binding_id=$1", [target.binding.id])).rows[0]?.settings).toEqual({ browser: "fixture-browser", allowed_urls: ["https://example.org/editor"], scope: "Save work drafts" });
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await session.getByRole("button", { name: "Experience learning", exact: true }).click();
  await expect(automation.getByLabel("Enable automatic learning and reuse")).toBeChecked();
  await automation.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath("automatic-browser-permission.png"), animations: "disabled" });
  await automation.getByRole("button", { name: "Stop browser automation" }).click();
  await expect(automation.getByRole("status")).toHaveText("Automation off or learning unavailable");
  await expect.poll(async () => (await schedulingDb.query("SELECT settings FROM browser_automation WHERE binding_id=$1", [target.binding.id])).rows[0]?.settings).toBeNull();
  await schedulingDb.query("UPDATE experience_policy SET enabled=FALSE,decision_settings=NULL WHERE binding_id=$1", [target.binding.id]);
  await dialog.getByLabel("Enable background learning").uncheck();
  await dialog.getByRole("button", { name: "Clear active revision", exact: true }).click();
  await expect.poll(async () => (await status()).policy.active_revision_id).toBeNull();
  const revisionDetails = dialog.locator(`[data-revision-id="${revision}"]`);
  await revisionDetails.locator("summary").click();
  await revisionDetails.getByRole("button", { name: "Restore this revision", exact: true }).click();
  await expect.poll(async () => (await status()).policy.active_revision_id).toBe(revision);
  await dialog.getByLabel("Automatically build tasks from work episodes").check();
  await expect(dialog.getByLabel("Task input", { exact: true })).toHaveCount(0);
  await dialog.getByRole("button", { name: "Save settings", exact: true }).click();
  await expect.poll(async () => (await status()).policy.optimization_settings.trace_cases).toBe(true);
  // The worker-to-report contract is covered by the gateway integration test;
  // this owned revision exercises report delivery and rendering in the dialog.
  const reportDb = await postgresQueryClient();
  await reportDb.query("UPDATE experience_revision SET validation=jsonb_set(validation,'{dataset}',$1::jsonb) WHERE id=$2 AND binding_id=$3", [JSON.stringify({
    version: "curation-ui-version", previous_version: "previous", total: 12, added: 3, updated: 2,
    withdrawn: 1, unevaluable: 2, duplicate_groups: 2, conflicting_groups: 1,
    categories: { math: 8, coding: 4 }, outcomes: { direct: 8, incomplete: 4 }, variants: 2,
  }), revision, target.binding.id]);
  await dialog.getByRole("button", { name: "Refresh history", exact: true }).click();
  const quality = revisionDetails.getByRole("region", { name: "Dataset quality report" });
  await expect(quality).toContainText("12 objectives");
  await expect(quality).toContainText("3 added · 2 updated · 1 withdrawn");
  await expect(quality).toContainText("incomplete: 4");
  await expect(quality).toContainText("2 reviewed training paraphrases");
  await expect(dialog.getByRole("region", { name: "Measured task difficulty" })).toContainText("No matching completed training trials yet");
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await send("Learning is disabled for this turn.");
  const after = JSON.parse(await readFile(path.join(target.workspace_path, ".fixture-native.json"), "utf8"));
  const last = after.claude.filter((row: { type: string; message: { content: { type: string }[] } }) => row.type === "user" && row.message.content[0].type === "text").at(-1);
  expect(last.message.content[0].text).toBe("Learning is disabled for this turn.");
});

for (const driver of ["claude_terminal", "codex_terminal"]) {
  for (const remote of [false, true]) {
    runtimeTest(`structured ${driver} on ${remote ? "device B" : "this computer"} preserves approvals across reload`, async ({ page, device }) => {
      const { company, host, headers, home } = device;
      const name = uniqueName("structured-dm");
      const fixture = path.join(home, `structured-${driver}`);
      await writeFile(fixture, await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/structured-cli.py")), { mode: 0o700 });
      const created = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: {
        name, instructions: "Verify the selected device workspace.", driver_type: driver,
        workspace_id: company.id, ...(remote ? { runtime_host_id: host.id } : {}),
      } });
      expect(created.status(), await created.text()).toBe(201);
      const agent = await created.json();
      const db = await postgresQueryClient();
      await db.query("UPDATE agent_runtime_bindings SET config_json = (config_json - 'model') || $1::jsonb WHERE id=$2", [JSON.stringify({ binary_path: fixture }), agent.binding.id]);
      await gotoDashboard(page);
      await page.getByRole("button", { name: "Select company" }).click();
      await page.locator(".company-dropdown-item").filter({ hasText: company.name }).locator(".company-dropdown-item-name").click();
      const open = async () => {
        const conversation = page.locator(".conv-item").filter({ hasText: name }).first();
        await expect(conversation).toBeVisible();
        await conversation.click();
        await expect(page.getByRole("region", { name: "Agent session" })).toBeVisible();
      };
      await open();
      const session = page.getByRole("region", { name: "Agent session" });
      await session.getByLabel("Message Agent").fill("Inspect the workspace");
      await expect(session.getByRole("button", { name: "Send", exact: true })).toBeEnabled();
      await session.getByRole("button", { name: "Send", exact: true }).click();
      await expect(session.getByRole("button", { name: "Allow once" })).toBeVisible();
      await expect(readFile(path.join(agent.workspace_path, "approved-on-device"))).rejects.toThrow();
      await page.reload();
      await open();
      await session.getByRole("button", { name: "Allow once" }).click();
      await expect(session.getByText("Verified workspace on selected device", { exact: true })).toBeVisible();
      await expect.poll(() => readFile(path.join(agent.workspace_path, "approved-on-device"), "utf8").catch(() => "")).toBe(agent.workspace_path);
      await session.locator("details summary").first().click();
      await expect(session.locator("details pre").first()).toContainText(agent.workspace_path);
      await session.getByLabel("Message Agent").fill("wait");
      await session.getByRole("button", { name: "Send", exact: true }).click();
      await session.getByRole("button", { name: "Stop", exact: true }).click();
      await expect(session.getByRole("status")).toHaveText("ready");
      await expect(session.getByRole("button", { name: "Stop", exact: true })).toHaveCount(0);
      await expect(session.getByRole("button", { name: "Reconnect", exact: true })).toHaveCount(0);
      const reopened = page.waitForResponse((response) =>
        new URL(response.url()).pathname.endsWith(`/runtime/bindings/${agent.binding.id}/session`)
        && response.request().method() === "POST",
      );
      await page.reload();
      await open();
      // The region mounts before its reopen request returns. Verify replay
      // after that owned request, not against the temporary connecting view.
      expect((await reopened).ok()).toBeTruthy();
      await expect(session.getByText("Verified workspace on selected device", { exact: true })).toHaveCount(1);
      const beforeTerminal = await (await page.request.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`, { headers })).json();
      await session.getByRole("button", { name:"Terminal", exact:true }).click();
      await expect(session.locator(".terminal-container > .xterm")).toHaveCount(1);
      await session.getByRole("button", { name:"Conversation", exact:true }).click();
      await expect(session.getByRole("status")).toHaveText("ready");
      await expect(session.getByText("Verified workspace on selected device", { exact: true })).toHaveCount(1);
      const afterTerminal = await (await page.request.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`, { headers })).json();
      expect(afterTerminal.session_id).toBe(beforeTerminal.session_id);
      const anonymous = await request.newContext();
      try {
        const unauthorized = await anonymous.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`);
        expect(unauthorized.status()).toBe(401);
      } finally {
        await anonymous.dispose();
      }
      const current = await (await page.request.get(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session`, { headers })).json();
      const stale = await page.request.post(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session/commands`, { headers, data: { action: "send", instance: randomUUID(), text: "stale request", submission_id: randomUUID() } });
      expect(stale.status()).toBe(409);
      const closed = await page.request.post(`${API_BASE}/v1/runtime/bindings/${agent.binding.id}/session/commands`, { headers, data: { action: "close", instance: current.instance } });
      expect(closed.ok(), await closed.text()).toBe(true);
    });
  }
}

runtimeTest("remote terminal activity records outcomes and byte counts without content", async ({ page, device }) => {
  const { home, company, host, headers, session } = device;
  await writeFile(path.join(home, "muse-target"), '#!/bin/sh\nexec /bin/cat\n', { mode: 0o700 });
  const response = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: {
    name: uniqueName("terminal-activity"), instructions: "Terminal transport fixture.",
    driver_type: "muse_terminal", workspace_id: company.id, runtime_host_id: host.id,
  } });
  expect(response.status(), await response.text()).toBe(201);
  const bindingId = (await response.json()).binding.id;
  const privateInput = uniqueName("not-for-activity-storage");
  const traceId = randomUUID();
  const inputHeaders = { ...headers, "x-trace-id": traceId };
  const inputUrl = `${API_BASE}/v1/terminals/${bindingId}/input`;
  // No PTY exists yet: this must be recorded as a failed attempt, not success.
  const failed = await page.request.post(inputUrl, { headers: inputHeaders, data: { data: privateInput } });
  expect(failed.ok()).toBe(false);
  expect((await page.request.post(`${API_BASE}/v1/terminals/${bindingId}/ensure`, { headers })).ok()).toBe(true);
  expect((await page.request.post(inputUrl, { headers: inputHeaders, data: { data: privateInput } })).ok()).toBe(true);
  const marker = uniqueName("terminal-wire");
  const url = `${API_BASE.replace(/^http/, "ws")}/v1/ws/terminals/${bindingId}?token=${encodeURIComponent(session.token)}`;
  await page.evaluate(({ url, marker }) => new Promise<void>((resolve, reject) => {
    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";
    let received = "";
    const timer = setTimeout(() => { socket.close(); reject(new Error("terminal echo not received")); }, 10000);
    socket.onopen = () => {
      socket.send(JSON.stringify({ type: "resize", cols: 90, rows: 30 }));
      socket.send(marker + "\r");
    };
    socket.onmessage = event => {
      received += new TextDecoder().decode(event.data);
      if (received.includes(marker)) socket.close();
    };
    socket.onerror = () => { clearTimeout(timer); reject(new Error("terminal socket failed")); };
    socket.onclose = () => { clearTimeout(timer); received.includes(marker) ? resolve() : reject(new Error("terminal closed before echo")); };
  }), { url, marker });
  const db = await postgresQueryClient();
  const records = () => db.query("SELECT action,metadata FROM audit_log WHERE actor_id=$1 AND target_id=$2 AND action LIKE 'terminal.%' ORDER BY created_at,id", [session.principal.id, bindingId]);
  await expect.poll(async () => (await records()).rows.filter(row => row.action === "terminal.detached").length).toBe(1);
  const rows = (await records()).rows;
  const finished = rows.filter(row => row.action === "terminal.submit_finished");
  expect(finished.map(row => row.metadata.outcome)).toEqual(["failed", "succeeded"]);
  for (const result of finished) {
    expect(result.metadata.trace_id).toBe(traceId);
    expect(rows.filter(row => row.action === "terminal.submit_started" && row.metadata.submission_id === result.metadata.submission_id)).toHaveLength(1);
  }
  const attached = rows.find(row => row.action === "terminal.attached")!;
  const detached = rows.find(row => row.action === "terminal.detached")!;
  expect(detached.metadata.attachment_id).toBe(attached.metadata.attachment_id);
  expect(detached.metadata.runtime_host_id).toBe(host.id);
  expect(detached.metadata.company_id).toBe(company.id);
  expect(detached.metadata.resize_count).toBe(1);
  expect(detached.metadata.input_bytes).toBe(Buffer.byteLength(marker + "\r"));
  expect(detached.metadata.output_bytes).toBeGreaterThan(0);
  expect(JSON.stringify(rows)).not.toContain(privateInput);
  expect(JSON.stringify(rows)).not.toContain(marker);
});

runtimeTest("remote terminals resolve the target executable and preserve explicit target paths", async ({ page, device }) => {
  const { home, company, host, headers } = device;
  const script = (marker: string) => `#!/bin/sh\nprintf '%s' '${marker}' > "$PWD/observed-binary"\nexec /bin/cat\n`;
  await writeFile(path.join(home, "bin", "muse"), script("wrong-path-command"), { mode: 0o700 });
  await writeFile(path.join(home, "muse-target"), script("target-environment"), { mode: 0o700 });
  const explicit = path.join(home, "muse-explicit");
  await writeFile(explicit, script("explicit-target"), { mode: 0o700 });
  const db = await postgresQueryClient();
  for (const mode of ["automatic", "explicit", "missing"]) {
    const response = await page.request.post(`${WEB_BASE}/api/agents/provision`, { data: { name: uniqueName(`binary-${mode}`), instructions: "Verify the target executable.", driver_type: "muse_terminal", workspace_id: company.id, runtime_host_id: host.id } });
    expect(response.status(), await response.text()).toBe(201);
    const created = await response.json();
    const stored = await db.query<{ config_json: Record<string, unknown> }>("SELECT config_json FROM agent_runtime_bindings WHERE id = $1", [created.binding.id]);
    expect(stored.rows[0].config_json.binary_path).toBeUndefined();
    if (mode !== "automatic") {
      await db.query("UPDATE agent_runtime_bindings SET config_json = config_json || $1::jsonb WHERE id = $2", [JSON.stringify({ binary_path: mode === "explicit" ? explicit : path.join(home, "not-installed") }), created.binding.id]);
    }
    const ensured = await page.request.post(`${API_BASE}/v1/terminals/${created.binding.id}/ensure`, { headers });
    if (mode === "missing") {
      expect(ensured.status()).toBe(500);
      await expect(readFile(path.join(created.workspace_path, "observed-binary"), "utf8")).rejects.toThrow();
    } else {
      expect(ensured.ok(), await ensured.text()).toBeTruthy();
      await expect.poll(() => readFile(path.join(created.workspace_path, "observed-binary"), "utf8").catch(() => "")).toBe(mode === "explicit" ? "explicit-target" : "target-environment");
    }
  }

  const workspace = path.join(home, "imported");
  const catalog = path.join(home, ".grok", "sessions", "project", "session");
  await mkdir(workspace);
  await mkdir(catalog, { recursive: true });
  const sessionId = randomUUID();
  await writeFile(path.join(catalog, "summary.json"), JSON.stringify({ info: { id: sessionId, cwd: workspace, title: "Imported target" } }));
  const imported = await page.request.post(`${API_BASE}/v1/workspace-sessions/import`, { headers, data: { company_id: company.id, runtime_host_id: host.id, workspace_path: workspace, sessions: [{ harness: "grok", native_session_id: sessionId, workspace_path: workspace }] } });
  if (process.env.CHORUZ_PLUGINS?.split(",").map(id => id.trim()).includes("grok")) {
    await writeFile(path.join(home, "grok-target"), script("target-environment"), { mode: 0o700 });
    expect(imported.ok(), await imported.text()).toBeTruthy();
    const bindingId = (await imported.json()).imported[0].binding_id;
    const binding = await db.query<{ config_json: Record<string, unknown> }>("SELECT config_json FROM agent_runtime_bindings WHERE id = $1", [bindingId]);
    expect(binding.rows[0].config_json.binary_path).toBeUndefined();
    const ensured = await page.request.post(`${API_BASE}/v1/terminals/${bindingId}/ensure`, { headers });
    expect(ensured.ok(), await ensured.text()).toBeTruthy();
    await expect.poll(() => readFile(path.join(workspace, "observed-binary"), "utf8").catch(() => "")).toBe("target-environment");
  } else {
    expect(imported.status(), await imported.text()).toBe(404);
    await expect(readFile(path.join(workspace, "observed-binary"))).rejects.toThrow();
  }
});

runtimeTest("remote Codex terminal uses the selected account on its own device", async ({ page, device }) => {
  test.setTimeout(90_000);
  const { session, headers, company, home, host } = device;
    const binary = path.join(home, "codex");
    await writeFile(binary, '#!/bin/sh\ncat "$CODEX_HOME/auth.json" > "$PWD/observed-auth"\nprintf "Account loaded\\n"\nexec cat\n', { mode: 0o700 });

    for (const profile of ["default", "isolated"]) {
      const accountId = randomUUID();
      const source = profile === "default" ? path.join(home, ".codex") : path.join(home, "accounts", accountId, "codex");
      const marker = `device-b-${profile}`;
      await mkdir(source, { recursive: true });
      await writeFile(path.join(source, "auth.json"), marker);
      const agent = await provisionAgent(page, session.token, uniqueName(`codex-${profile}`), { workspaceId: company.id });
      const workspace = path.join(home, profile);
      await mkdir(workspace);
      // Seed a selected binding; the contract starts at terminal ensure, not login
      // or provisioning. Only the external CLI is substituted below.
      const db = await postgresQueryClient();
      await db.query("INSERT INTO harness_account (id, company_id, runtime_host_id, driver_type, name, profile_kind, status) VALUES ($1, $2, $3, 'codex_terminal', $4, $4, 'active')", [accountId, company.id, host.id, profile]);
      const rows = await db.query("UPDATE agent_runtime_bindings SET driver_type = 'codex_terminal', workspace_path = $1, external_session_id = NULL, config_json = (config_json - 'model') || $2::jsonb WHERE agent_principal_id = $3 RETURNING id", [workspace, JSON.stringify({ runtime_host_id: host.id, binary_path: binary, harness_account_id: accountId, harness_account_profile_kind: profile }), agent.agentId]);
      expect(rows.rows).toHaveLength(1);
      const bindingId = rows.rows[0].id;
      const ensured = await page.request.post(`${API_BASE}/v1/terminals/${bindingId}/ensure`, { headers });
      expect(ensured.ok(), await ensured.text()).toBeTruthy();
      await expect.poll(() => readFile(path.join(workspace, "observed-auth"), "utf8").catch(() => "")).toBe(marker);
      expect(await realpath(path.join(home, "runtime", "codex-homes", String(bindingId), "auth.json"))).toBe(path.join(source, "auth.json"));
    }
});

test.describe("Terminal view (PTY)", () => {
  let terminalAgentName: string;
  let terminalAgentId: string;
  let terminalBindingId: string;

  test.beforeEach(async ({ page }) => {
    await login(page);
    terminalAgentName = uniqueName("terminal-agent");
    const agent = await provisionAgent(page, "", terminalAgentName);
    terminalAgentId = agent.agentId;
    const binary = path.join(agent.workspacePath, "fixture-cli");
    await writeFile(binary, await readFile(path.resolve("../../crates/choruz-host-runtime/tests/fixtures/structured-cli.py")), {mode:0o700});
    const db = await postgresQueryClient();
    const bindings = await db.query("UPDATE agent_runtime_bindings SET config_json=(config_json - 'model') || $1::jsonb WHERE agent_principal_id=$2 RETURNING id", [JSON.stringify({binary_path:binary}), agent.agentId]);
    expect(bindings.rows).toHaveLength(1);
    terminalBindingId = bindings.rows[0].id;
    await gotoDashboard(page);
  });

  /* ---------------------------------------------------------------------- */
  /*  Helpers                                                                */
  /* ---------------------------------------------------------------------- */

  async function selectDirectAgentConv(
    page: import("@playwright/test").Page,
  ): Promise<void> {
    const item = page.locator(".conv-item").filter({ hasText: terminalAgentName }).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    const ready = page.waitForResponse(async response =>
      new URL(response.url()).pathname.endsWith(`/runtime/bindings/${terminalBindingId}/session`)
      && response.ok() && (await response.json()).status === "ready");
    await item.click();
    await ready;
    await expect(page.getByRole("region", {name:"Agent session"}).getByRole("status")).toHaveText("ready");
    await page.getByRole("button", {name:"Terminal", exact:true}).click();
    const term = page.locator(".terminal-container, .xterm, .xterm-screen").first();
    await expect(term).toBeVisible();
  }

  /* ---------------------------------------------------------------------- */
  /*  Terminal rendering                                                     */
  /* ---------------------------------------------------------------------- */

  test("should render terminal for direct agent conversations", async ({
    page,
  }) => {
    await selectDirectAgentConv(page);
    const term = page.locator(".terminal-container, .xterm, .xterm-screen");
    await expect(term.first()).toBeVisible();
  });

  test("fits the terminal screen to the conversation pane", async ({ page }) => {
    await selectDirectAgentConv(page);
    await expect.poll(async () => {
      const screen = await page.locator(".xterm-screen").boundingBox();
      const pane = await page.locator(".terminal-container").boundingBox();
      return screen && pane ? screen.width / pane.width : 0;
    }).toBeGreaterThan(0.9);
  });

  test("uses the light terminal palette on the light page", async ({ page }) => {
    await selectDirectAgentConv(page);
    const screenshot = await page.locator(".xterm-screen").screenshot();
    const rgb = await page.evaluate(async (bytes) => {
      const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], {type:"image/png"}));
      const canvas = new OffscreenCanvas(image.width, image.height);
      const context = canvas.getContext("2d")!;
      context.drawImage(image, 0, 0);
      return Array.from(context.getImageData(Math.floor(image.width / 2), Math.floor(image.height / 2), 1, 1).data).slice(0, 3);
    }, [...screenshot]);
    expect(rgb).toEqual([250, 249, 245]);
  });

  /* ---------------------------------------------------------------------- */
  /*  WebSocket connection                                                   */
  /* ---------------------------------------------------------------------- */

  test("should attempt WebSocket connection for terminal", async ({
    page,
  }) => {
    const frames: string[] = [];
    page.on("websocket", (socket) => {
      if (socket.url().includes("/ws/terminals")) socket.on("framereceived", ({payload}) => frames.push(payload.toString()));
    });

    await selectDirectAgentConv(page);
    await expect.poll(() => frames.some((frame) => frame.includes("Raw terminal connected"))).toBe(true);
  });

  /* ---------------------------------------------------------------------- */
  /*  Reconnection                                                           */
  /* ---------------------------------------------------------------------- */

  test("reconnects the terminal after a dropped socket", async ({
    page,
  }) => {
    const sockets: string[] = [];
    page.on("websocket", (socket) => {
      if (socket.url().includes("/ws/terminals/")) sockets.push(socket.url());
    });
    await page.evaluate(() => {
      const NativeSocket = window.WebSocket;
      window.WebSocket = class extends NativeSocket {
        constructor(url: string | URL, protocols?: string | string[]) {
          super(url, protocols);
          if (String(url).includes("/ws/terminals/")) {
            (window as unknown as { closeTestTerminal: () => void }).closeTestTerminal = () => this.close(1000, "test disconnect");
          }
        }
      };
    });
    await selectDirectAgentConv(page);
    await expect.poll(() => sockets.length).toBe(1);
    await page.evaluate(() => (window as unknown as { closeTestTerminal: () => void }).closeTestTerminal());
    await expect.poll(() => sockets.length, {timeout:10000}).toBeGreaterThan(1);
  });

  /* ---------------------------------------------------------------------- */
  /*  Terminal focus                                                          */
  /* ---------------------------------------------------------------------- */

  test("should focus terminal on click", async ({ page }) => {
    await selectDirectAgentConv(page);
    const terminal = page.locator(".xterm, .terminal-container").first();
    await terminal.click();
    // Terminal should receive focus
    const hasFocus = await page.evaluate(() => {
      const active = document.activeElement;
      return active?.closest(".xterm") !== null ||
        active?.closest(".terminal-container") !== null;
    });
    expect(hasFocus).toBe(true);
  });

  /* ---------------------------------------------------------------------- */
  /*  No console errors                                                      */
  /* ---------------------------------------------------------------------- */

  test("should not produce terminal-related console errors", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    await selectDirectAgentConv(page);
    await page.waitForTimeout(3000);
    const termErrors = errors.filter(
      (e) =>
        e.includes("xterm") ||
        e.includes("Terminal") ||
        e.includes("WebSocket"),
    );
    // Some WebSocket errors are expected if the backend terminal is not running
    // But there should be no xterm initialization errors
    const initErrors = termErrors.filter(
      (e) =>
        e.includes("Cannot read") ||
        e.includes("undefined") ||
        e.includes("is not a function"),
    );
    expect(initErrors).toHaveLength(0);
  });

  /* ---------------------------------------------------------------------- */
  /*  Terminal not shown for group chats                                     */
  /* ---------------------------------------------------------------------- */

  test("shows PTY for direct terminal agent chats and composer for groups with that agent", async ({
    page,
  }) => {
    const { token, principal } = await login(page);
    const groupName = uniqueName("terminal-group");
    const group = await createGroup(page, token, principal.id, groupName, [terminalAgentId]);
    const groupMessage = `normal transcript marker ${Date.now()}`;
    await sendMessage(page, token, principal.id, group.id, groupMessage);
    await gotoDashboard(page);

    const directItem = page.locator(".conv-item").filter({ hasText: terminalAgentName }).first();
    await expect(directItem).toBeVisible({ timeout: 15_000 });
    await selectDirectAgentConv(page);
    await expect(page.locator(".terminal-container:visible").first()).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator(".xterm:visible, .xterm-screen:visible").first()).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator(".terminal-container:visible > .xterm")).toHaveCount(1);
    await expect(page.locator(".chat-input-row textarea").first()).toBeHidden();

    const groupItem = page.locator(".conv-item").filter({ hasText: group.name }).first();
    await expect(groupItem).toBeVisible({ timeout: 15_000 });
    await groupItem.click();
    const messagesArea = page.locator(".messages-area").first();
    await expect(messagesArea).toBeVisible({ timeout: 10_000 });
    await expect(messagesArea.getByText(groupMessage)).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".chat-input-row textarea").first()).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator(".terminal-container:visible")).toHaveCount(0);
  });
});

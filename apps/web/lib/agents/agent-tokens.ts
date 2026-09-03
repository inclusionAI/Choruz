import { promises as fs } from "node:fs";
import { randomUUID } from "node:crypto";
import path from "node:path";

/**
 * Resolve the project-root `.choruz-runtime` directory.
 *
 * When the Next.js dev server runs via `pnpm --dir apps/web dev` the CWD is
 * `apps/web/`, so the monorepo root is `../../`.  The env var
 * `CHORUZ_RUNTIME_DIR` takes priority when set.
 */
function runtimeDir(): string {
  if (process.env.CHORUZ_RUNTIME_DIR) {
    return process.env.CHORUZ_RUNTIME_DIR;
  }
  return path.resolve(process.cwd(), "..", "..", ".choruz-runtime");
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function withFileLock<T>(
  lockPath: string,
  fn: () => Promise<T>,
): Promise<T> {
  const deadline = Date.now() + 10_000;

  while (true) {
    try {
      await fs.mkdir(lockPath, { mode: 0o700 });
      break;
    } catch (err) {
      const code = (err as NodeJS.ErrnoException).code;
      if (code !== "EEXIST" || Date.now() >= deadline) {
        throw err;
      }
      await sleep(10);
    }
  }

  try {
    return await fn();
  } finally {
    await fs.rm(lockPath, { recursive: true, force: true });
  }
}

/** Read or initialise `agent_tokens.json` and add a token entry. */
export async function persistAgentToken(
  agentPrincipalId: string,
  secret: string,
): Promise<void> {
  const tokensPath = path.join(runtimeDir(), "agent_tokens.json");
  await fs.mkdir(path.dirname(tokensPath), { recursive: true });

  await withFileLock(`${tokensPath}.lock`, async () => {
    let tokens: Record<string, string> = {};
    try {
      const raw = await fs.readFile(tokensPath, "utf-8");
      tokens = JSON.parse(raw) as Record<string, string>;
    } catch {
      // File does not exist yet or is invalid -- start fresh.
    }

    tokens[agentPrincipalId] = secret;

    // Atomic write via per-call temp file + rename.
    const tmpPath = `${tokensPath}.${process.pid}.${randomUUID()}.tmp`;
    await fs.writeFile(tmpPath, JSON.stringify(tokens, null, 2) + "\n", {
      encoding: "utf-8",
      mode: 0o600,
    });
    await fs.rename(tmpPath, tokensPath);
    await fs.chmod(tokensPath, 0o600); // ensure owner-only after rename
  });
}

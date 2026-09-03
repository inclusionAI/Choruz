import { mkdir, writeFile, access } from "node:fs/promises";
import path from "node:path";

const root = process.cwd();
const typesDir = path.join(root, ".next", "types");
const routesDts = path.join(typesDir, "routes.d.ts");
const routesJs = path.join(typesDir, "routes.js");

try {
  await access(routesDts);
} catch {
  process.exit(0);
}

try {
  await access(routesJs);
} catch {
  await mkdir(typesDir, { recursive: true });
  await writeFile(routesJs, "export {};\n", "utf8");
}

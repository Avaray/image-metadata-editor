#!/usr/bin/env node

const { spawnSync } = require("child_process");
const path = require("path");
const os = require("os");
const fs = require("fs");

const platform = os.platform();
const arch = os.arch();
const packageName = `@avaray/ime-${platform}-${arch}`;
const binName = platform === "win32" ? "ime.exe" : "ime";

function findBinary() {
  // Strategy 1: resolve via the optional dependency's package.json
  try {
    const pkgJsonPath = require.resolve(`${packageName}/package.json`);
    const candidate = path.join(path.dirname(pkgJsonPath), binName);
    if (fs.existsSync(candidate)) return candidate;
  } catch (_) {}

  // Strategy 2: walk up node_modules directories (handles hoisting quirks)
  let dir = __dirname;
  for (let i = 0; i < 5; i++) {
    const candidate = path.join(dir, "node_modules", packageName, binName);
    if (fs.existsSync(candidate)) return candidate;
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }

  return null;
}

const exePath = findBinary();
if (!exePath) {
  console.error(
    `Unsupported platform or missing binary package: ${packageName}`
  );
  console.error("Please ensure the optional dependency was installed:");
  console.error(`  npm install ${packageName}`);
  console.error(`  bun add ${packageName}`);
  process.exit(1);
}

const result = spawnSync(exePath, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(result.error);
  process.exit(1);
}
process.exit(result.status ?? 0);

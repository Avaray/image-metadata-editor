#!/usr/bin/env node

const { spawnSync } = require('child_process');
const path = require('path');
const os = require('os');

// Determine the correct platform package
const platform = os.platform();
const arch = os.arch();
const packageName = `@avaray/ime-${platform}-${arch}`;

let exePath;
try {
  // Resolve the path to the binary in the optional dependency
  const pkgPath = require.resolve(`${packageName}/package.json`);
  const binName = platform === 'win32' ? 'ime.exe' : 'ime';
  exePath = path.join(path.dirname(pkgPath), binName);
} catch (e) {
  console.error(`Unsupported platform or missing binary package: ${packageName}`);
  console.error('Please ensure the optional dependency was installed correctly.');
  process.exit(1);
}

// Execute the binary with all passed arguments
const args = process.argv.slice(2);
const result = spawnSync(exePath, args, { stdio: 'inherit' });

if (result.error) {
  console.error(result.error);
  process.exit(1);
}
process.exit(result.status);

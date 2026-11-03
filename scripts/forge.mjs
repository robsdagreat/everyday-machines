#!/usr/bin/env node
// Runs the forge binary, building it first if it is missing or older than its sources.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const bin = join(root, "target", "release", process.platform === "win32" ? "forge.exe" : "forge");

const newest = (dir) =>
  readdirSync(dir, { withFileTypes: true }).reduce((m, e) => {
    const p = join(dir, e.name);
    return Math.max(m, e.isDirectory() ? newest(p) : statSync(p).mtimeMs);
  }, 0);

const sources = Math.max(newest(join(root, "src")), statSync(join(root, "Cargo.toml")).mtimeMs);
if (!existsSync(bin) || statSync(bin).mtimeMs < sources) {
  const build = spawnSync("cargo", ["build", "--release", "--locked"], { cwd: root, stdio: "inherit" });
  if (build.error?.code === "ENOENT") {
    console.error("error: cargo not found. Install Rust from https://rustup.rs");
    process.exit(1);
  }
  if (build.status !== 0) process.exit(build.status ?? 1);
}

// Run from the repo root so forge finds forge.toml, whichever directory pnpm was invoked from.
const run = spawnSync(bin, ["--root", root, ...process.argv.slice(2)], { stdio: "inherit" });
process.exit(run.status ?? 1);

#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

function findProjectRoot() {
  let current = process.cwd();
  while (current !== path.dirname(current)) {
    if (
      fs.existsSync(path.join(current, "package.json")) &&
      fs.existsSync(path.join(current, "src-tauri", "tauri.conf.json"))
    ) {
      return current;
    }
    current = path.dirname(current);
  }
  throw new Error("Could not locate the project root.");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    stdio: options.capture ? ["ignore", "pipe", "pipe"] : "inherit",
  });
  if (result.status !== 0) {
    const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(
      `${command} ${args.join(" ")} failed${detail ? `:\n${detail}` : ""}`
    );
  }
  return result.stdout || "";
}

function listFilesRecursive(dir) {
  if (!fs.existsSync(dir)) {
    return [];
  }
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...listFilesRecursive(fullPath));
    } else if (entry.isFile()) {
      files.push(fullPath);
    }
  }
  return files;
}

function signPath(target) {
  run("codesign", ["--force", "--sign", "-", "--timestamp=none", target], {
    capture: true,
  });
}

function signAppBundle() {
  const projectRoot = findProjectRoot();
  const appPath = path.join(
    projectRoot,
    "src-tauri",
    "target",
    "release",
    "bundle",
    "macos",
    "Stream Hub.app"
  );
  if (!fs.existsSync(appPath)) {
    return;
  }

  const frameworksDir = path.join(appPath, "Contents", "Frameworks");
  for (const file of listFilesRecursive(frameworksDir)) {
    signPath(file);
  }

  const executable = path.join(appPath, "Contents", "MacOS", "stream-hub");
  if (fs.existsSync(executable)) {
    signPath(executable);
  }

  run(
    "codesign",
    ["--force", "--deep", "--sign", "-", "--timestamp=none", appPath],
    { capture: true }
  );
  run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", appPath], {
    capture: true,
  });
  console.log(`Signed macOS app bundle at ${appPath}.`);
}

function main() {
  if (process.platform !== "darwin") {
    return;
  }
  signAppBundle();
}

main();

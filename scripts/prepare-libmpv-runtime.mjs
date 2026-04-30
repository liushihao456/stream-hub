#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

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

function listFilesRecursive(dir) {
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

function findRuntimeDirFromPath() {
  const pathEnv = process.env.PATH || "";
  for (const dir of pathEnv.split(path.delimiter)) {
    if (!dir) {
      continue;
    }
    const candidate = path.join(dir, "libmpv-2.dll");
    if (fs.existsSync(candidate)) {
      return dir;
    }
  }
  return null;
}

function copyWindowsRuntime(projectRoot) {
  const sourceDir =
    process.env.STREAM_HUB_MPV_RUNTIME_DIR ||
    process.env.LIBMPV_RUNTIME_DIR ||
    process.env.MPV_RUNTIME_DIR ||
    process.env.LIBMPV_DIR ||
    findRuntimeDirFromPath();

  if (!sourceDir || !fs.existsSync(sourceDir)) {
    throw new Error(
      "Windows libmpv runtime was not found. Set STREAM_HUB_MPV_RUNTIME_DIR to an extracted mpv runtime directory containing libmpv-2.dll."
    );
  }

  const dlls = listFilesRecursive(sourceDir).filter((file) =>
    file.toLowerCase().endsWith(".dll")
  );
  if (!dlls.some((file) => path.basename(file).toLowerCase() === "libmpv-2.dll")) {
    throw new Error(`libmpv-2.dll was not found in ${sourceDir}.`);
  }

  const targetDir = path.join(
    projectRoot,
    "src-tauri",
    "target",
    "libmpv-runtime",
    "windows"
  );
  fs.rmSync(targetDir, { recursive: true, force: true });
  fs.mkdirSync(targetDir, { recursive: true });

  const copied = new Set();
  for (const dll of dlls) {
    const fileName = path.basename(dll);
    if (copied.has(fileName.toLowerCase())) {
      continue;
    }
    fs.copyFileSync(dll, path.join(targetDir, fileName));
    copied.add(fileName.toLowerCase());
  }

  console.log(`Staged ${copied.size} Windows libmpv DLL(s) from ${sourceDir}.`);
}

function main() {
  if (process.platform !== "win32" && process.env.STREAM_HUB_PREPARE_MPV_WINDOWS !== "1") {
    return;
  }

  copyWindowsRuntime(findProjectRoot());
}

main();

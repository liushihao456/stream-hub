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

function parseOtoolLibraries(file) {
  const output = run("otool", ["-L", file], { capture: true });
  return output
    .split("\n")
    .slice(1)
    .map((line) => {
      const match = line.match(/^\s+(.+?)\s+\(compatibility version/);
      return match ? match[1] : null;
    })
    .filter(Boolean);
}

function parseRpaths(file) {
  const output = run("otool", ["-l", file], { capture: true });
  const rpaths = [];
  const regex = /cmd LC_RPATH[\s\S]*?path (.+?) \(offset \d+\)/g;
  let match;
  while ((match = regex.exec(output))) {
    rpaths.push(match[1]);
  }
  return rpaths;
}

function isSystemLibrary(libraryPath) {
  return (
    libraryPath.startsWith("/usr/lib/") ||
    libraryPath.startsWith("/System/Library/") ||
    libraryPath.startsWith("@executable_path/") ||
    libraryPath.startsWith("@loader_path/")
  );
}

function isBundledLibraryCandidate(libraryPath) {
  return path.isAbsolute(libraryPath) && !isSystemLibrary(libraryPath);
}

function findMpvDylib(binary) {
  const linked = parseOtoolLibraries(binary).find((libraryPath) =>
    /(^|\/)libmpv\.\d+\.dylib$/.test(libraryPath)
  );
  if (linked && path.isAbsolute(linked) && fs.existsSync(linked)) {
    return linked;
  }

  const explicit = process.env.LIBMPV_DYLIB_PATH;
  if (explicit && fs.existsSync(explicit)) {
    return explicit;
  }

  const candidates = [
    "/opt/homebrew/opt/mpv/lib/libmpv.2.dylib",
    "/usr/local/opt/mpv/lib/libmpv.2.dylib",
    "/opt/homebrew/lib/libmpv.2.dylib",
    "/usr/local/lib/libmpv.2.dylib",
  ];
  const candidate = candidates.find((item) => fs.existsSync(item));
  if (!candidate) {
    throw new Error("Could not find libmpv. Install mpv with Homebrew or set LIBMPV_DYLIB_PATH.");
  }
  return candidate;
}

function findReleaseBinary(projectRoot) {
  const targetTriple = process.env.CARGO_BUILD_TARGET || process.env.TAURI_ENV_TARGET_TRIPLE;
  const candidates = [];
  if (targetTriple) {
    candidates.push(
      path.join(projectRoot, "src-tauri", "target", targetTriple, "release", "stream-hub")
    );
  }
  candidates.push(path.join(projectRoot, "src-tauri", "target", "release", "stream-hub"));

  const binary = candidates.find((candidate) => fs.existsSync(candidate));
  if (!binary) {
    throw new Error(`Could not find release binary. Checked: ${candidates.join(", ")}`);
  }
  return binary;
}

function copyLibraryClosure(rootLibrary, frameworkDir) {
  fs.rmSync(frameworkDir, { recursive: true, force: true });
  fs.mkdirSync(frameworkDir, { recursive: true });

  const copiedByOriginal = new Map();
  const copiedByBasename = new Map();
  const queue = [rootLibrary];

  while (queue.length > 0) {
    const source = queue.shift();
    if (!source || copiedByOriginal.has(source) || !fs.existsSync(source)) {
      continue;
    }

    const fileName = path.basename(source);
    const destination = path.join(frameworkDir, fileName);
    fs.copyFileSync(source, destination);
    fs.chmodSync(destination, 0o755);
    copiedByOriginal.set(source, { fileName, destination });
    copiedByBasename.set(fileName, { fileName, destination });

    for (const dependency of parseOtoolLibraries(source)) {
      if (isBundledLibraryCandidate(dependency) && fs.existsSync(dependency)) {
        queue.push(dependency);
      }
    }
  }

  return { copiedByOriginal, copiedByBasename };
}

function changeInstallName(file, oldName, newName) {
  if (oldName === newName) {
    return;
  }
  run("install_name_tool", ["-change", oldName, newName, file], { capture: true });
}

function rewriteDylibs(copiedByOriginal, copiedByBasename) {
  for (const [original, copied] of copiedByOriginal) {
    run("install_name_tool", ["-id", `@rpath/${copied.fileName}`, copied.destination], {
      capture: true,
    });
    for (const dependency of parseOtoolLibraries(copied.destination)) {
      const dependencyCopy =
        copiedByOriginal.get(dependency) || copiedByBasename.get(path.basename(dependency));
      if (!dependencyCopy) {
        continue;
      }
      changeInstallName(
        copied.destination,
        dependency,
        `@loader_path/${dependencyCopy.fileName}`
      );
    }
  }
}

function rewriteAppBinary(binary, copiedByOriginal, copiedByBasename) {
  for (const dependency of parseOtoolLibraries(binary)) {
    const dependencyCopy =
      copiedByOriginal.get(dependency) || copiedByBasename.get(path.basename(dependency));
    if (!dependencyCopy) {
      continue;
    }
    changeInstallName(
      binary,
      dependency,
      `@executable_path/../Frameworks/${dependencyCopy.fileName}`
    );
  }

  const frameworkRpath = "@executable_path/../Frameworks";
  if (!parseRpaths(binary).includes(frameworkRpath)) {
    run("install_name_tool", ["-add_rpath", frameworkRpath, binary], { capture: true });
  }
}

function bundleMacRuntime() {
  const projectRoot = findProjectRoot();
  const binary = findReleaseBinary(projectRoot);
  const mpvDylib = findMpvDylib(binary);
  const frameworkDir = path.join(
    projectRoot,
    "src-tauri",
    "target",
    "libmpv-runtime",
    "macos",
    "Frameworks"
  );

  const { copiedByOriginal, copiedByBasename } = copyLibraryClosure(mpvDylib, frameworkDir);
  rewriteDylibs(copiedByOriginal, copiedByBasename);
  rewriteAppBinary(binary, copiedByOriginal, copiedByBasename);

  console.log(`Bundled ${copiedByOriginal.size} macOS libmpv dylib(s) into ${frameworkDir}.`);
}

function main() {
  const platform = process.env.TAURI_ENV_PLATFORM || process.platform;
  if (platform !== "macos" && platform !== "darwin") {
    return;
  }

  bundleMacRuntime();
}

main();

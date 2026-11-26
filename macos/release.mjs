import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: projectRoot, stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.status ?? result.signal})`);
  }
  return result;
}

function buildRelease(args) {
  assert.equal(process.platform, "darwin", "Use macos/release.sh on macOS");
  const pkg = JSON.parse(readFileSync(path.join(projectRoot, "package.json"), "utf8"));
  const version = pkg.version;

  const result = run(
    "/usr/bin/arch",
    [
      "-arm64",
      "cargo",
      "build",
      "--release",
      "--locked",
      ...args,
      "--target-dir",
      path.join(projectRoot, "target/macos-build"),
      "--message-format=json-render-diagnostics",
    ],
    { encoding: "utf8", stdio: ["inherit", "pipe", "inherit"], maxBuffer: 32 * 1024 * 1024 }
  );

  const artifacts = result.stdout
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line))
    .filter(
      (item) =>
        item.reason === "compiler-artifact" &&
        item.target?.name === "agy-plus" &&
        item.executable
    );
  assert.equal(artifacts.length, 1, "Expected one agy-plus executable from Cargo");

  const destination = path.join(projectRoot, "target/release/agy-plus");
  mkdirSync(path.dirname(destination), { recursive: true });
  const stage = mkdtempSync(path.join(path.dirname(destination), ".macos-release-"));
  try {
    const candidate = path.join(stage, "agy-plus");
    copyFileSync(artifacts[0].executable, candidate);
    chmodSync(candidate, 0o755);
    run("/bin/bash", [path.join(projectRoot, "macos/sign.sh"), candidate]);
    const archInfo = run("/usr/bin/file", ["-b", candidate], {
      encoding: "utf8",
      stdio: "pipe",
    }).stdout.trim();
    assert.match(archInfo, /Mach-O/, "Release must be a macOS Mach-O executable");
    renameSync(candidate, destination);

    const portableDir = path.join(projectRoot, "target/portable");
    mkdirSync(portableDir, { recursive: true });
    const archive = path.join(portableDir, `agy-plus-${version}-macos-arm64.tar.gz`);
    run("/usr/bin/tar", [
      "-czf",
      archive,
      "-C",
      path.dirname(destination),
      path.basename(destination),
    ]);
    console.log(`Built and signed ${version} (${archInfo}): ${destination}`);
    console.log(`Portable archive: ${archive}`);
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}

buildRelease(process.argv.slice(2));

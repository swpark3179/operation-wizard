#!/usr/bin/env node
// Bumps the app version (major/minor/patch) and writes it into every file that
// carries a version number: package.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml.
// Usage: node bump-version.mjs <major|minor|patch> <currentVersion>
// Prints the new version to stdout.

import { readFileSync, writeFileSync } from "node:fs";

const [, , bumpType, currentVersion] = process.argv;

if (!["major", "minor", "patch"].includes(bumpType)) {
  console.error(`Unknown bump type: ${bumpType}`);
  process.exit(1);
}

const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec((currentVersion ?? "").trim());
if (!match) {
  console.error(`Cannot parse version: ${currentVersion}`);
  process.exit(1);
}

let [major, minor, patch] = match.slice(1).map(Number);
if (bumpType === "major") {
  major += 1;
  minor = 0;
  patch = 0;
} else if (bumpType === "minor") {
  minor += 1;
  patch = 0;
} else {
  patch += 1;
}
const newVersion = `${major}.${minor}.${patch}`;

const pkgPath = "package.json";
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
pkg.version = newVersion;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");

const tauriConfPath = "src-tauri/tauri.conf.json";
const tauriConf = JSON.parse(readFileSync(tauriConfPath, "utf8"));
tauriConf.version = newVersion;
writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + "\n");

const cargoPath = "src-tauri/Cargo.toml";
const cargoToml = readFileSync(cargoPath, "utf8");
const updatedCargoToml = cargoToml.replace(
  /^version = "[^"]*"/m,
  `version = "${newVersion}"`,
);
writeFileSync(cargoPath, updatedCargoToml);

console.log(newVersion);

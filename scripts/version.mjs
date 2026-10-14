// Usage: node scripts/version.mjs <patch|minor|major|x.y.z>
// package.json is the source of truth; tauri.conf.json reads it directly,
// and this script mirrors it into Cargo.toml and Cargo.lock.
import { readFileSync, writeFileSync } from "node:fs";

const pkgPath = new URL("../package.json", import.meta.url);
const cargoPath = new URL("../src-tauri/Cargo.toml", import.meta.url);
const lockPath = new URL("../src-tauri/Cargo.lock", import.meta.url);

const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
const arg = process.argv[2];
const semver = /^\d+\.\d+\.\d+$/;

let next;
if (!arg) {
  console.log(pkg.version);
  process.exit(0);
} else if (semver.test(arg)) {
  next = arg;
} else if (["patch", "minor", "major"].includes(arg)) {
  const [maj, min, pat] = pkg.version.split(".").map(Number);
  next =
    arg === "major" ? `${maj + 1}.0.0` : arg === "minor" ? `${maj}.${min + 1}.0` : `${maj}.${min}.${pat + 1}`;
} else {
  console.error("usage: version.mjs <patch|minor|major|x.y.z>");
  process.exit(1);
}

pkg.version = next;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");

const cargo = readFileSync(cargoPath, "utf8");
writeFileSync(cargoPath, cargo.replace(/^version = ".*"$/m, `version = "${next}"`));

const lock = readFileSync(lockPath, "utf8");
writeFileSync(
  lockPath,
  lock.replace(/(name = "proxalens"\nversion = )".*"/, `$1"${next}"`),
);

console.log(`version -> ${next}`);

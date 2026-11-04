import * as esbuild from "esbuild";
import { readFileSync, writeFileSync, readdirSync, statSync } from "fs";
import { dirname, join, relative } from "path";
import { fileURLToPath } from "url";

const here = dirname(fileURLToPath(import.meta.url));

const result = await esbuild.build({
  absWorkingDir: here,
  entryPoints: ["src/main.js"],
  bundle: true,
  format: "iife",
  target: ["es2018"],
  outfile: "bundle.js",
  metafile: true,
  legalComments: "none",
});

function relativize(value) {
  if (typeof value === "string") {
    if (value === here) return ".";
    if (value.startsWith(here + "/")) return relative(here, value);
    return value;
  }
  if (Array.isArray(value)) return value.map(relativize);
  if (value && typeof value === "object") {
    const out = {};
    for (const [key, child] of Object.entries(value)) {
      out[relativize(key)] = relativize(child);
    }
    return out;
  }
  return value;
}

writeFileSync(join(here, "meta.json"), JSON.stringify(relativize(result.metafile), null, 2) + "\n");

const bundle = readFileSync(join(here, "bundle.js"), "utf8");
const forbidden = ["/Users/", "/home/", here, "OffscreenCanvas", "navigator.locks"];
for (const needle of forbidden) {
  if (bundle.includes(needle)) {
    console.error("bundle.js contains forbidden text: " + needle);
    process.exit(1);
  }
}

function collectPackages(dir, found) {
  let entries = [];
  try {
    entries = readdirSync(dir);
  } catch {
    return;
  }
  for (const name of entries) {
    if (name === ".bin" || name.startsWith(".")) continue;
    const path = join(dir, name);
    let info;
    try {
      info = statSync(path);
    } catch {
      continue;
    }
    if (!info.isDirectory()) continue;
    if (name.startsWith("@") || name === "node_modules") {
      collectPackages(path, found);
      continue;
    }
    try {
      statSync(join(path, "package.json"));
      found.push(path);
    } catch {
      // Not a package.
    }
    collectPackages(join(path, "node_modules"), found);
  }
}

const packages = [];
collectPackages(join(here, "node_modules"), packages);
const allowed = new Set(["MIT", "MIT OR Apache-2.0", "(MIT OR Apache-2.0)"]);
for (const dir of packages) {
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  const license = pkg.license || "";
  const name = pkg.name || dir;
  if (name.startsWith("@codemirror/") && !allowed.has(license)) {
    console.error(name + " license is " + (license || "missing"));
    process.exit(1);
  }
}

console.log("bundle.js " + bundle.length + " bytes, " + packages.length + " MIT packages");

// TypeScript side of the eqts bridge: drives the Rust parity probes through
// the generated Bun adapter and diffs them against the C++ driver output.
//
// Run (normally via the rx4 `ts_bridge_check` tool):
//   bun ts/harness.ts <cpp-driver> <vectors.txt>

import { readFileSync } from "node:fs";

import { harnessVersion, nullBattery, parityProbe } from "../dist/bun/index.js";

const [cppDriver, vectorsPath] = process.argv.slice(2);
if (!cppDriver || !vectorsPath) {
  console.error("usage: bun harness.ts <cpp-driver> <vectors.txt>");
  process.exit(2);
}

if (harnessVersion() !== 1) {
  console.error("harness version mismatch");
  process.exit(2);
}

// C++ side: run the driver over the committed vectors (prints its null
// battery first, then one line per probe).
const cpp = Bun.spawnSync([cppDriver, vectorsPath]);
if (cpp.exitCode !== 0) {
  console.error("C++ driver failed:", cpp.stderr.toString());
  process.exit(1);
}
const cppLines = cpp.stdout.toString().split("\n").filter((l) => l.length > 0);

// Rust side, through eqts: null battery first (driver order), then one probe
// per vector line with the driver's argv semantics (missing columns -> "").
const vectorLines = readFileSync(vectorsPath, "utf8")
  .split("\n")
  .map((l) => l.trim())
  .filter((l) => l.length > 0 && !l.startsWith("#"));
const rustLines = [
  ...nullBattery(),
  ...vectorLines.map((line) => {
    const t = line.split(/\s+/);
    return parityProbe(t[0], t[1] ?? "", t[2] ?? "");
  }),
];

if (cppLines.length !== rustLines.length) {
  console.error(
    `line count mismatch: cpp=${cppLines.length} rust(eqts)=${rustLines.length}`,
  );
  process.exit(1);
}

let mismatches = 0;
for (let i = 0; i < cppLines.length; i++) {
  if (cppLines[i] !== rustLines[i]) {
    mismatches++;
    if (mismatches <= 5) {
      console.error(`mismatch at line ${i}:\n  cpp : ${cppLines[i]}\n  rust: ${rustLines[i]}`);
    }
  }
}
if (mismatches > 0) {
  console.error(`${mismatches} mismatch(es)`);
  process.exit(1);
}
console.log(
  `eqts bridge: ${cppLines.length} lines identical (C++ driver vs Rust probes via eqts/Bun)`,
);

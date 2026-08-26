import { rmSync } from "node:fs";
import path from "node:path";

const obsoleteCommands = [
  "cadgen-ax-preview-render",
  "cadgen-ax-artifact-export",
  "cadgen-ax-evaluate-structural"
];

for (const profile of ["debug", "release"]) {
  const dir = path.join("src-tauri", "target", profile);
  for (const command of obsoleteCommands) {
    for (const suffix of ["", ".exe", ".dSYM"]) {
      rmSync(path.join(dir, `${command}${suffix}`), {
        force: true,
        recursive: true
      });
    }
  }
}

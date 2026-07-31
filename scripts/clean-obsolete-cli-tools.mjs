import { rmSync } from "node:fs";
import path from "node:path";

const obsoleteCommands = [
  "cadastrophe-preview-render",
  "cadastrophe-artifact-export",
  "cadastrophe-evaluate-structural",
  "cadastrophe-vlm-submit"
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

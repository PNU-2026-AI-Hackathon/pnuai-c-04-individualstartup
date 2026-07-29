import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import test from "node:test";

const execFileAsync = promisify(execFile);

test("openscad-wasm helper renders boolean, rotate, and fn source to STL and mesh", async () => {
  const dir = await mkdtemp(join(tmpdir(), "cadastrophe-openscad-wasm-"));
  const sourcePath = join(dir, "fixture.scad");
  await writeFile(
    sourcePath,
    `
$fn = 20;
difference() {
  union() {
    cube([10, 8, 4], center=true);
    rotate([0, 0, 45]) translate([0, 0, 3]) cylinder(h=4, r=3, center=true);
  }
  translate([0, 0, -4]) cylinder(h=12, r=1.25, center=true);
}
`,
    "utf8"
  );

  try {
    const { stdout } = await execFileAsync("node", ["scripts/openscad-render.mjs", sourcePath], {
      cwd: process.cwd(),
      maxBuffer: 10 * 1024 * 1024
    });
    const result = JSON.parse(stdout);
    assert.equal(result.diagnostics.ok, true);
    assert.equal(typeof result.stlBase64, "string");
    assert.ok(Buffer.from(result.stlBase64, "base64").byteLength > 1000);
    assert.ok(result.mesh.vertices.length > 0);
    assert.ok(result.mesh.indices.length > 0);
    assert.match(result.stlSha256, /^[a-f0-9]{64}$/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

import assert from "node:assert/strict";
import test from "node:test";
import { stlExportDefaultFileName } from "../ui/src/runtime/artifactExport";

test("STL export default name preserves a useful session name", () => {
  assert.equal(stlExportDefaultFileName("Rear axle bracket"), "Rear axle bracket.stl");
  assert.equal(stlExportDefaultFileName("기어 어셈블리"), "기어 어셈블리.stl");
});

test("STL export default name removes path and platform-reserved characters", () => {
  assert.equal(stlExportDefaultFileName("../drive/gear:*?"), "..-drive-gear---.stl");
  assert.equal(stlExportDefaultFileName("  ...  "), "cadastrophe-model.stl");
});

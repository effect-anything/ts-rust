// node --test js/test: the content-mapper API wrappers, against a fake client.
import test from "node:test";
import assert from "node:assert/strict";
import { mappedFiles, isUserWritten, expansionAt } from "../api.mjs";

const info = {
  fileName: "/p/a.sts", mapped: true, contentMapper: "@effect-x/tsc",
  dependencies: ["/p/m.sts"], syntaxImports: [],
  expansions: [{ virtualStart: 11, virtualEnd: 17, originalStart: 54, originalEnd: 62, label: "macro `twice`" }],
  regions: [
    { virtualStart: 0, virtualEnd: 11, kind: "verbatim", originalStart: 43, originalEnd: 54 },
    { virtualStart: 11, virtualEnd: 17, kind: "generated", originalStart: 54, originalEnd: 62 },
  ],
};

function fakeProgram(async) {
  const calls = [];
  const answers = {
    getMappedFileInfo: info,
    getMappedFilesInfo: [info, null],
    getOriginalPositions: [{ position: 46, fidelity: "exact" }],
    getVirtualPositions: [[{ position: 12, fidelity: "exact" }, { position: 15, fidelity: "exact" }]],
  };
  const client = {
    apiRequest(method, params) {
      calls.push([method, params]);
      return async ? Promise.resolve(answers[method]) : answers[method];
    },
  };
  return { program: { client, snapshotId: 7, id: "p1" }, calls };
}

test("sync client: values, ids and batching", () => {
  const { program, calls } = fakeProgram(false);
  const api = mappedFiles({ program });
  assert.equal(api.getMappedFileInfo("/p/a.sts").fileName, "/p/a.sts");
  assert.deepEqual(calls[0], ["getMappedFileInfo", { snapshot: 7, project: "p1", file: "/p/a.sts" }]);
  assert.deepEqual(api.getMappedFileInfo(["/p/a.sts", "/p/x.ts"]).map((x) => x?.fileName), ["/p/a.sts", undefined]);
  assert.equal(calls[1][1].files.length, 2);
  assert.equal(api.getMappedFilesInfo().length, 1);
  assert.deepEqual(api.getOriginalPosition("/p/a.sts", 3), { position: 46, fidelity: "exact" });
  assert.deepEqual(api.getVirtualPositions("/p/a.sts", 60).map((p) => p.position), [12, 15]);
  assert.deepEqual(calls.at(-1)[1].positions, [60]);
});

test("async client: promises", async () => {
  const { program } = fakeProgram(true);
  const api = mappedFiles(program);
  assert.equal((await api.getMappedFileInfo("/p/a.sts")).dependencies[0], "/p/m.sts");
  assert.equal((await api.getOriginalPosition("/p/a.sts", [3])).length, 1);
});

test("helpers", () => {
  assert.equal(isUserWritten(info, 2, 5), true);
  assert.equal(isUserWritten(info, 12), false);
  assert.equal(expansionAt(info, 12).label, "macro `twice`");
  assert.equal(expansionAt(info, 3), undefined);
  assert.throws(() => mappedFiles({}), TypeError);
});

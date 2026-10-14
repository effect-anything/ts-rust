// "@effect-x/tsc/unstable/mapped": typed wrappers for the content-mapper
// methods tsc --api adds (crates/ts_goport/src/api/ext_api.rs). They work
// with the upstream unstable client, sync or async: pass a `Program` (or a
// `Project`, whose program is used) from "@effect-x/tsc/unstable/sync" or
// "@effect-x/tsc/unstable/async". Results arrive as the client returns
// them: a value for the sync client, a Promise for the async one.
//
// Positions are UTF-16 code units, as in the rest of the API.

function target(programOrProject) {
  const program = programOrProject?.program ?? programOrProject;
  const client = program?.client;
  if (!client || typeof client.apiRequest !== "function" || program.snapshotId === undefined) {
    throw new TypeError("mappedFiles(program): expected a Program or Project from @effect-x/tsc/unstable");
  }
  return { client, snapshot: program.snapshotId, project: program.id };
}

function then(value, map) {
  return value && typeof value.then === "function" ? value.then(map) : map(value);
}

const orUndefined = (value) => (value === null ? undefined : value);

/** Content-mapper information for one program's files. */
export function mappedFiles(programOrProject) {
  const { client, snapshot, project } = target(programOrProject);
  const request = (method, params) => client.apiRequest(method, { snapshot, project, ...params });
  return {
    getMappedFileInfo(fileOrFiles) {
      if (Array.isArray(fileOrFiles)) {
        return then(request("getMappedFilesInfo", { files: fileOrFiles }), (list) => list.map(orUndefined));
      }
      return then(request("getMappedFileInfo", { file: fileOrFiles }), orUndefined);
    },
    getMappedFilesInfo() {
      return then(request("getMappedFilesInfo", {}), (list) => list.filter(Boolean));
    },
    getOriginalPosition(file, positionOrPositions) {
      const many = Array.isArray(positionOrPositions);
      const positions = many ? positionOrPositions : [positionOrPositions];
      return then(request("getOriginalPositions", { file, positions }), (list) => {
        if (list == null) return undefined;
        return many ? list : list[0];
      });
    },
    getVirtualPositions(file, positionOrPositions) {
      const many = Array.isArray(positionOrPositions);
      const positions = many ? positionOrPositions : [positionOrPositions];
      return then(request("getVirtualPositions", { file, positions }), (list) => {
        if (list == null) return undefined;
        return many ? list : list[0];
      });
    },
  };
}

/** Whether a virtual range lies entirely in user-written text. */
export function isUserWritten(info, start, end = start) {
  return info.regions.some((r) => r.kind === "verbatim" && r.virtualStart <= start && end <= r.virtualEnd);
}

/** The innermost expansion containing a virtual range, if any. */
export function expansionAt(info, start, end = start) {
  let best;
  for (const e of info.expansions) {
    if (e.virtualStart <= start && end <= e.virtualEnd && (!best || e.virtualEnd - e.virtualStart < best.virtualEnd - best.virtualStart)) {
      best = e;
    }
  }
  return best;
}

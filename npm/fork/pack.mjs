// Writes the linux-x64 npm package set in Go's layout at the pin: `typescript` (the JS
// launcher and the JS API) and `@typescript/typescript-linux-x64` (the native tsc and the
// lib files). scripts/fork/npm-pack.sh runs it (the fork copy of npm/pack.mjs and
// scripts/goport/npm-pack.sh: upstream's stay as on main); see there for the usage.
//
// It follows Herebyfile.mjs `buildNativePreviewPackages` of the Go checkout with the
// release profile "typescript" (publishAsTypescript) for the current platform only, as
// Go's local build does. With --native-bin it adds one thing to the main package: the
// postinstall npm/install.js (as lib/install.js), which rewrites bin/tsc on POSIX as a sh and JS
// polyglot: sh runs the native tsc without Node, and Node runs Go's launcher (see there).
//
// PORT: not in Go. `--name @effect-x/tsc` writes the port's own package set from the same input:
// `@effect-x/tsc` (bin `tsc`, npm/getExePath.js as lib/getExePath.js, npm/tsc-readme.md) and one
// platform package `@effect-x/tsc-<os>-<arch>` per --exe. The default name `typescript` is Go's set.
// The port's package has its own --package-version.
//
// PORT: @effect-x/tsc also ships tsc-ext (crates/ts_ext_syntax): --ext <os>-<arch>=<dir> names a dir
// holding that platform's tsc-ext binary (tsc-ext, tsc-ext.exe on Windows) and its Node addon
// (tsc-ext.node). The --ext platforms must be exactly the --exe platforms. Each platform package then
// has lib/tsc, lib/tsc-ext and lib/tsc-ext.node; the main package has the bins `tsc` and `tsc-ext`
// (bin/tsc-ext is a Node launcher that runs the platform's lib/tsc-ext), the tsconfig
// `contentMappers` manifest (exec ./bin/tsc-ext; tsc starts the platform package's lib/tsc-ext
// in its place when it is installed, by convention), and the Node API
// (crates/ts_ext_syntax/js as lib/ext, the subpath export "./ext"; the root export stays
// TypeScript's). The tsc keeps reporting the TypeScript version
// (--version): the compiler matches `typesVersions` against it, so a tsc that reported 0.1.0 would
// pick the typings that packages keep for TypeScript 5.6 and older. The main package records it as
// `tscVersion`, for the postinstall check.
//
// usage: node npm/pack.mjs --layout <typescript|typescript-go> --go-dir <dir> --exe <tsc>...
//          --libs <dir> --dist <dir> --version <v> --git-head <sha> --out <dir> [--native-bin]
//          [--name <typescript|@effect-x/tsc>] [--package-version <v>]
//
// --version is the version the tsc reports. --package-version is the npm version (default
// --version).
//
// --exe is <os>-<arch>=<tsc> (Node's process.platform and process.arch, for example
// darwin-arm64=<path>), or a plain <tsc> for linux-x64. Repeat it for more platforms. It is not
// run here, so a cross-built tsc works.
//
// --layout is the pin layout (scripts/upstream/pin.py). "typescript" (microsoft/TypeScript, pin N
// on): --go-dir is <repo>/tsc, the input is <repo>/packages/typescript (it already has bin/tsc and
// lib/tsc.js), and LICENSE.txt and NOTICE.txt are at <repo>. "typescript-go": the input is
// <go-dir>/_packages/native-preview (bin/tsgo, lib/tsgo.js), with LICENSE and NOTICE.txt at <go-dir>.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
    options: {
        layout: { type: "string" },
        "go-dir": { type: "string" },
        exe: { type: "string", multiple: true },
        ext: { type: "string", multiple: true },
        name: { type: "string", default: "typescript" },
        "package-version": { type: "string" },
        libs: { type: "string" },
        dist: { type: "string" },
        version: { type: "string" },
        "git-head": { type: "string" },
        out: { type: "string" },
        "native-bin": { type: "boolean", default: false },
    },
    strict: true,
});
for (const name of ["layout", "go-dir", "exe", "libs", "dist", "version", "git-head", "out"]) {
    if (!args[name]) throw new Error(`missing --${name}`);
}
const { layout, "go-dir": goDir, libs, dist, version, "git-head": gitHead, out, name } = args;
if (layout !== "typescript" && layout !== "typescript-go") throw new Error(`unknown --layout ${layout}`);
if (name !== "typescript" && name !== "@effect-x/tsc") throw new Error(`unknown --name ${name}`);
const asTypescript = name === "typescript";
const binName = "tsc";
const scope = asTypescript ? "" : name.slice(0, name.indexOf("/"));
const unscoped = asTypescript ? name : name.slice(name.indexOf("/") + 1);
const readmeStem = asTypescript ? "" : unscoped;
const packageVersion = args["package-version"] ?? version;
const atN = layout === "typescript";
const root = atN ? path.dirname(goDir) : goDir;
const inputDir = atN ? path.join(root, "packages", "typescript") : path.join(goDir, "_packages", "native-preview");
const tsLicenseFile = path.join(root, atN ? "LICENSE.txt" : "LICENSE");
const tsNoticeFile = path.join(root, "NOTICE.txt");

// Go: Herebyfile.mjs getPlatforms. PORT: the platforms are the --exe values, not only the current
// platform as in Go's local build.
const platforms = args.exe.map(value => {
    const match = value.match(/^([a-z0-9]+)-([a-z0-9]+)=(.+)$/);
    const [nodeOs, nodeArch, exe] = match ? match.slice(1) : ["linux", "x64", value];
    return {
        nodeOs,
        nodeArch,
        exe,
        packageName: asTypescript
            ? `@typescript/typescript-${nodeOs}-${nodeArch}`
            : `${scope}/${unscoped}-${nodeOs}-${nodeArch}`,
    };
});

// PORT: tsc-ext per platform (@effect-x/tsc only), the same platform set as tsc.
const extDirs = new Map((args.ext ?? []).map(value => {
    const match = value.match(/^([a-z0-9]+-[a-z0-9]+)=(.+)$/);
    if (!match) throw new Error(`--ext wants <os>-<arch>=<dir>, not ${value}`);
    return [match[1], match[2]];
}));
if (asTypescript && extDirs.size > 0) throw new Error("--ext is for --name @effect-x/tsc only");
if (!asTypescript) {
    const tscSet = platforms.map(p => `${p.nodeOs}-${p.nodeArch}`).sort().join(" ");
    const extSet = [...extDirs.keys()].sort().join(" ");
    if (tscSet !== extSet) {
        throw new Error(`the tsc-ext platforms (--ext: ${extSet || "none"}) must be the tsc platforms (--exe: ${tscSet})`);
    }
}
const extExeName = nodeOs => (nodeOs === "win32" ? "tsc-ext.exe" : "tsc-ext");

// Go: Herebyfile.mjs getPublishTag (publishAsTypescript). Go's nativePreviewReleaseVersion is
// undefined at both layouts, so a version with no dev, beta or rc part is refused.
// PORT: @effect-x/tsc has no such rule; `npm publish --tag` picks its tag.
function publishTag() {
    const match = version.match(/-(dev|beta|rc)(?:[.-]|$)/);
    if (match?.[1]) return match[1] === "dev" ? "next" : match[1];
    throw new Error(`Refusing to publish 'typescript' with the latest tag from non-release version ${version}.`);
}

// Go: Herebyfile.mjs stripSourceConditions and stripConditionsFromValue
function stripConditions(value) {
    if (value == null || typeof value !== "object") return value;
    delete value["@typescript/source"];
    for (const key of Object.keys(value)) value[key] = stripConditions(value[key]);
    const keys = Object.keys(value);
    return keys.length === 1 && keys[0] === "default" ? value.default : value;
}

const writeJson = (file, value) => fs.writeFileSync(file, JSON.stringify(value, undefined, 4));

// Go: Herebyfile.mjs buildNativePreviewPackages, inputPackageJson with publishAsTypescript.
const input = JSON.parse(fs.readFileSync(path.join(inputDir, "package.json"), "utf8"));
input.version = packageVersion;
delete input.private;
input.files = [...new Set([...(input.files ?? []), "NOTICE.txt"])];
input.bin = asTypescript
    ? { [binName]: `./bin/${binName}` }
    : { [binName]: `./bin/${binName}`, "tsc-ext": "./bin/tsc-ext" };
if (asTypescript) {
    input.description = "TypeScript is a language for application scale JavaScript development";
    input.homepage = "https://www.typescriptlang.org/";
    input.keywords = ["TypeScript", "Microsoft", "compiler", "language", "javascript"];
    input.bugs = { url: "https://github.com/microsoft/TypeScript/issues" };
    input.repository = { type: "git", url: "https://github.com/microsoft/TypeScript.git" };
}
else {
    // PORT: the port's own package. The port is MIT. The JS launcher, the JS API and the lib files
    // are TypeScript's, unchanged (Apache-2.0). See portNotice below.
    input.license = "MIT AND Apache-2.0";
    // The publisher of @effect-x/tsc; the port's copyright stays in LICENSE and NOTICE.
    input.author = "effect-anything";
    input.description = "A Rust port of the TypeScript 7 compiler";
    input.keywords = ["typescript", "tsc", "compiler", "rust"];
    // npm trusted publishing (the release workflow) needs repository.url to name the repo that
    // publishes. The platform packages copy it.
    input.homepage = "https://github.com/effect-anything/tsc";
    input.bugs = { url: "https://github.com/effect-anything/tsc/issues" };
    input.repository = { type: "git", url: "git+https://github.com/effect-anything/tsc.git" };
}
delete input.scripts;
delete input.devDependencies;
for (const field of ["exports", "imports"]) input[field] = stripConditions(input[field]);
input.gitHead = gitHead;
input.publishConfig = asTypescript ? { access: "public", tag: publishTag() } : { access: "public" };

fs.rmSync(out, { recursive: true, force: true });

// The main package (`typescript`, or `@effect-x/tsc`). The dir name has no scope.
const mainDir = path.join(out, unscoped);
const here = path.dirname(fileURLToPath(import.meta.url));
// npm/fork/: the repo root is two levels up; install.js is upstream's npm/install.js.
const repoRoot = path.dirname(path.dirname(here));
const npmDir = path.join(repoRoot, "npm");

// PORT: @effect-x/tsc ships the port's MIT LICENSE, and a NOTICE.txt with the licenses and notices of
// the code it ports or copies (the repo's NOTICE.md and licenses/). Go's set keeps TypeScript's.
function portNotice() {
    const part = (title, file) => [`${"=".repeat(78)}\n${title}\n${"=".repeat(78)}\n`, fs.readFileSync(file, "utf8")];
    return [
        fs.readFileSync(path.join(repoRoot, "NOTICE.md"), "utf8"),
        ...part("TypeScript license", tsLicenseFile),
        ...part("TypeScript third-party notices", tsNoticeFile),
        ...part("Go license", path.join(repoRoot, "licenses", "Go-LICENSE.txt")),
        ...part("Unicode license", path.join(repoRoot, "licenses", "Unicode-LICENSE.txt")),
    ].join("\n");
}
function writeLicense(dir) {
    if (asTypescript) {
        fs.copyFileSync(tsLicenseFile, path.join(dir, "LICENSE"));
        fs.copyFileSync(tsNoticeFile, path.join(dir, "NOTICE.txt"));
    }
    else {
        fs.copyFileSync(path.join(repoRoot, "LICENSE"), path.join(dir, "LICENSE"));
        fs.writeFileSync(path.join(dir, "NOTICE.txt"), portNotice());
    }
}
const mainPackage = {
    ...input,
    name,
    ...(asTypescript ? {} : { tscVersion: version }),
    optionalDependencies: Object.fromEntries(platforms.map(p => [p.packageName, packageVersion])),
};
if (atN) {
    // Go copies the whole input but node_modules and dist; its filter sees the path from the repo.
    fs.cpSync(inputDir, mainDir, {
        recursive: true,
        filter: src => {
            const p = path.posix.join("packages/typescript", path.relative(inputDir, src).split(path.sep).join("/"));
            return !p.endsWith("/node_modules") && !p.includes("/dist");
        },
    });
}
else {
    for (const entry of ["bin", "lib", "vendor"]) {
        fs.cpSync(path.join(inputDir, entry), path.join(mainDir, entry), { recursive: true });
    }
    fs.rmSync(path.join(mainDir, "bin", "tsgo"));
    fs.renameSync(path.join(mainDir, "lib", "tsgo.js"), path.join(mainDir, "lib", "tsc.js"));
}
fs.cpSync(dist, path.join(mainDir, "dist"), { recursive: true });
fs.rmSync(path.join(mainDir, "bin"), { recursive: true, force: true });
fs.mkdirSync(path.join(mainDir, "bin"));
fs.writeFileSync(path.join(mainDir, "bin", binName), '#!/usr/bin/env node\nimport "../lib/tsc.js";\n');
fs.chmodSync(path.join(mainDir, "bin", binName), 0o755);
if (asTypescript) {
    fs.copyFileSync(path.join(inputDir, "typescript-package-readme.md"), path.join(mainDir, "README.md"));
}
else {
    fs.copyFileSync(path.join(here, `${readmeStem}-readme.md`), path.join(mainDir, "README.md"));
    fs.copyFileSync(path.join(here, "getExePath.js"), path.join(mainDir, "lib", "getExePath.js"));
    // PORT: tsc-ext. The launcher runs lib/tsc-ext of the platform package with this process's
    // stdio, so it also serves as the content mapper tsc starts (typescript.contentMapper.exec).
    fs.writeFileSync(
        path.join(mainDir, "bin", "tsc-ext"),
        [
            "#!/usr/bin/env node",
            'import { spawnSync } from "node:child_process";',
            'import getExePath from "../lib/getExePath.js";',
            "",
            'const result = spawnSync(getExePath("tsc-ext"), process.argv.slice(2), { stdio: "inherit" });',
            "if (result.error) {",
            '    console.error(`tsc-ext: ${result.error.message}`);',
            "    process.exit(1);",
            "}",
            "if (result.signal) process.kill(process.pid, result.signal);",
            "process.exit(result.status ?? 1);",
            "",
        ].join("\n"),
    );
    fs.chmodSync(path.join(mainDir, "bin", "tsc-ext"), 0o755);
    const extJs = path.join(repoRoot, "crates", "ts_ext_syntax", "js");
    fs.cpSync(extJs, path.join(mainDir, "lib", "ext"), {
        recursive: true,
        filter: source => path.basename(source) !== "test",
    });
    // tsc starts the platform package's native lib/tsc-ext directly: for a one-entry exec
    // ./bin/<name>, the port's tsc looks for <package>-<platform>-<arch>/lib/<name> next to the
    // mapper package (crates/ts_goport/src/frontend/tsoptions/content_mappers.rs `native_exec`).
    // exec, the Node launcher, is the fallback when that package is not installed, and the
    // command for hosts without that convention.
    mainPackage.typescript = {
        contentMapper: {
            exec: ["./bin/tsc-ext"],
        },
    };
    // The root export stays TypeScript's (the version); the Node API is "@effect-x/tsc/ext".
    mainPackage.exports = {
        ...mainPackage.exports,
        "./ext": { types: "./lib/ext/index.d.ts", default: "./lib/ext/index.cjs" },
        // The bundler plugin: "@effect-x/tsc/unplugin" and "@effect-x/tsc/unplugin/<bundler>".
        "./unplugin": { types: "./lib/ext/unplugin.d.mts", default: "./lib/ext/unplugin.mjs" },
        "./unplugin/*": { types: "./lib/ext/unplugin/*.d.mts", default: "./lib/ext/unplugin/*.mjs" },
    };
    mainPackage.dependencies = { ...mainPackage.dependencies, unplugin: "^3.3.0" };
}
if (args["native-bin"]) {
    // PORT: not in Go. The native bin on POSIX (npm/install.js).
    mainPackage.scripts = { postinstall: "node lib/install.js" };
    fs.copyFileSync(path.join(npmDir, "install.js"), path.join(mainDir, "lib", "install.js"));
}
writeJson(path.join(mainDir, "package.json"), mainPackage);
writeLicense(mainDir);

// The platform packages: the lib files and the native tsc in lib/.
for (const { nodeOs, nodeArch, exe, packageName } of platforms) {
    const platformDir = path.join(out, `${unscoped}-${nodeOs}-${nodeArch}`);
    const platformPackage = {
        ...input,
        bin: undefined,
        files: ["lib", "NOTICE.txt"],
        imports: undefined,
        dependencies: undefined,
        name: packageName,
        os: [nodeOs],
        cpu: [nodeArch],
        exports: { "./package.json": "./package.json" },
    };
    fs.cpSync(libs, path.join(platformDir, "lib"), { recursive: true });
    // getExePath.js looks for lib/tsc.exe on Windows.
    const exeName = nodeOs === "win32" ? "tsc.exe" : "tsc";
    fs.copyFileSync(exe, path.join(platformDir, "lib", exeName));
    fs.chmodSync(path.join(platformDir, "lib", exeName), 0o755);
    const extDir = extDirs.get(`${nodeOs}-${nodeArch}`);
    if (extDir) {
        for (const file of [extExeName(nodeOs), "tsc-ext.node"]) {
            fs.copyFileSync(path.join(extDir, file), path.join(platformDir, "lib", file));
        }
        fs.chmodSync(path.join(platformDir, "lib", extExeName(nodeOs)), 0o755);
    }
    writeJson(path.join(platformDir, "package.json"), platformPackage);
    writeLicense(platformDir);
    fs.writeFileSync(
        path.join(platformDir, "README.md"),
        [
            `# \`${packageName}\``,
            "",
            `This package provides ${nodeOs}-${nodeArch} support for [${name}](https://www.npmjs.com/package/${name}).`,
        ].join("\n") + "\n",
    );
}

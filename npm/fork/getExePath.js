// lib/getExePath.js of the @effect-x/tsc package set (npm/pack.mjs --name @effect-x/tsc). It
// replaces Go's getExePath.js, which finds `@typescript/<name>-<platform>-<arch>`. The bin launcher
// (lib/tsc.js), the JS API and the postinstall (lib/install.js) import it as `#getExePath`.
//
// The native tsc of package <scope>/<name> is lib/tsc in <scope>/<name>-<platform>-<arch>.
// getExePath("tsc-ext") is lib/tsc-ext there (the bin/tsc-ext launcher).
import fs from "node:fs";
import module from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

export default function getExePath(exeName = "tsc") {
    const __dirname = path.dirname(fileURLToPath(import.meta.url));
    const { name } = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8"));
    const platformPackageName = `${name}-${process.platform}-${process.arch}`;
    if (!platformPackageName.startsWith("@")) {
        throw new Error(`Unable to resolve a platform package for ${name}.`);
    }
    let exeDir;
    try {
        const packageJson = typeof import.meta.resolve === "function"
            ? fileURLToPath(import.meta.resolve(`${platformPackageName}/package.json`))
            : module.createRequire(import.meta.url).resolve(`${platformPackageName}/package.json`);
        exeDir = path.join(path.dirname(packageJson), "lib");
    }
    catch {
        throw new Error(`Unable to resolve ${platformPackageName}. Either your platform is unsupported, or you are missing the package on disk.`);
    }
    const exe = path.join(exeDir, process.platform === "win32" ? `${exeName}.exe` : exeName);
    if (!fs.existsSync(exe)) {
        throw new Error(`Executable not found: ${exe}`);
    }
    return exe;
}

import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { TransformSession } from "ext-syntax";

const session = TransformSession.build();
const src = "src";
const out = "generated";
mkdirSync(out, { recursive: true });

for (const name of readdirSync(src)) {
  if (!/\.(ts|tsx|sts|stsx)$/.test(name) || name.endsWith(".d.ts")) continue;
  const file = session.transform(join(src, name), readFileSync(join(src, name), "utf8"));
  const outName = name.replace(/\.stsx$/, ".tsx").replace(/\.sts$/, ".ts");
  writeFileSync(join(out, outName), file.code);
  writeFileSync(join(out, `${outName}.map`), file.sourceMap);
}

writeFileSync(
  join(out, "tsconfig.json"),
  JSON.stringify(
    {
      compilerOptions: {
        strict: true,
        target: "esnext",
        module: "esnext",
        moduleResolution: "bundler",
        jsx: "react-jsx",
        rootDir: ".",
        outDir: "../dist",
        skipLibCheck: true,
      },
      files: ["main.ts", "view.tsx", "logic.ts", "panel.tsx", "overload.d.ts", "jsx-runtime.d.ts"],
    },
    null,
    2,
  ),
);
for (const name of ["overload.d.ts", "jsx-runtime.d.ts"]) {
  writeFileSync(join(out, name), readFileSync(join(src, name)));
}

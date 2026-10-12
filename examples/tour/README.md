# Tour

The four extensions in one program.

| file | extension |
| --- | --- |
| `logic.sts` | Sweetener `unless`, calls a `.tsx` file |
| `panel.stsx` | Sweetener file, then `"use gpu"` and JSX |
| `view.tsx` | TypeGPU vertex and fragment, unmarked `View` |
| `calc.ts` | `"use tsover"` |
| `main.ts` | ordinary TypeScript. Imports the other four |

Sweetener runs first on `.sts` and `.stsx`. TypeGPU and tsover run on the expanded text. `.sts` imports are rewritten to extensionless specifiers before `tsc`.

```sh
npm run build
```

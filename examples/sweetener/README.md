# Sweetener

`.sts` is not TypeScript. The content mapper expands it, then `tsc` sees `logic.ts`.

`unless (n < 0)` becomes `if (!(n < 0))`. `main.ts` is ordinary TypeScript and calls `step`.

```sh
npm run build
```

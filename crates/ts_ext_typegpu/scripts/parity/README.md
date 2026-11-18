# TypeGPU parity harness

Compares `Transformer` (TS source -> tsover + TypeGPU -> oxc strip) against
`tsc` emit with the hook, and scores source maps token by token.

    PARITY_NODE_MODULES=/path/with/oxc-transform,@jridgewell node compare.cjs <project>
    PARITY_NODE_MODULES=... node mapq.cjs <files...>
    PARITY_NODE_MODULES=... node tscmapq.cjs <outDir>

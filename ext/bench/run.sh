#!/usr/bin/env bash
# Expansion benchmarks. Release builds; numbers go to stdout as Markdown.
#
#   ext/bench/run.sh [copies]        # copies of bench.stsx's body (default 4)
#
# 1. expand_bench (in-process): cold / modules-warm / memo-hit / build
#    on bench.stsx, a synthetic file of `copies` bodies, and the largest
#    Sweetener cases.
# 2. tsc-ext sweetener --expand --map (process, protocol and v3 map included).
# 3. vite build of ext/examples/vite.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
copies="${1:-4}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cd "$root"
cargo build --release -q -p ts_ext_sweetener --example expand_bench
cargo build --release -q -p ts_ext_syntax --bin tsc-ext

cp ext/bench/fixture/*.sts ext/bench/fixture/bench.stsx "$work/"
python3 - "$work" "$copies" <<'PY'
import sys, pathlib
work, copies = pathlib.Path(sys.argv[1]), int(sys.argv[2])
lines = (work / "bench.stsx").read_text().split("\n")
head = 0
while head < len(lines) and (lines[head].startswith("import ") or not lines[head].strip()):
    head += 1
body = "\n".join(lines[head:])
(work / "big.stsx").write_text("\n".join(lines[:head]) + "\n" + "\n".join([body] * copies))
PY

largest=$(for f in crates/ts_ext_sweetener/cases/*/input.sts*; do
  printf '%s %s\n' "$(wc -c <"$f")" "$f"; done | sort -rn | head -5 | awk '{print $2}')

echo "## In-process expansion (release, median of 5)"
echo
# shellcheck disable=SC2086
target/release/examples/expand_bench "$work/bench.stsx" "$work/big.stsx" $largest

echo
echo "## tsc-ext sweetener --expand --map (release process, median of 5)"
echo
echo "| file | median s |"
echo "| --- | ---: |"
for f in "$work/bench.stsx" "$work/big.stsx"; do
  python3 - "$root/target/release/tsc-ext" "$f" <<'PY'
import subprocess, sys, time, statistics, os
tool, f = sys.argv[1], sys.argv[2]
times = []
for _ in range(5):
    t = time.perf_counter()
    subprocess.run([tool, "sweetener", "--expand", f, "--map"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    times.append(time.perf_counter() - t)
print(f"| {os.path.basename(f)} | {statistics.median(times):.3f} |")
PY
done

echo
echo "## vite build (ext/examples/vite)"
echo
cd ext/examples/vite
python3 - <<'PY'
import subprocess, time, statistics
times = []
for _ in range(3):
    t = time.perf_counter()
    subprocess.run(["npx", "vite", "build"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    times.append(time.perf_counter() - t)
print(f"median of 3: {statistics.median(times):.2f} s (runs: {', '.join(f'{x:.2f}' for x in times)})")
PY

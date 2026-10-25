#!/usr/bin/env bash
# 渲染 samples/ 下的示范工程为同名 .wav。
#
#   scripts/render_samples.sh            # 全部分组
#   scripts/render_samples.sh effects    # 只渲染某个分组
#
# 用环境变量 MUSER 指定二进制（默认 target/debug/muser）。
# plugins/ 需要 `cargo build -p muser-cli --features host` 构建的 muser；
# 依赖本机插件或音色库的工程，缺文件时会报错并跳过，不影响其它工程。
#
# 插件状态文件（.muserstate）与插件版本、平台绑定，不入库：每个 *.state.json
# 描述 {"plugin", "params", "output"}，渲染前先用 `muser plugin save-state` 现场生成。
# 工程里的相对路径都相对仓库根目录。
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MUSER="${MUSER:-$ROOT/target/debug/muser}"
GROUP="${1:-}"
cd "$ROOT"

if [ ! -x "$MUSER" ]; then
  echo "找不到 muser 二进制：$MUSER（先 cargo build -p muser-cli，或设置 MUSER）" >&2
  exit 1
fi

for r in samples/${GROUP:-*}/*.state.json; do
  [ -e "$r" ] || continue
  if python3 - "$r" "$MUSER" <<'PY'
import json, subprocess, sys
d = json.load(open(sys.argv[1]))
cmd = [sys.argv[2], "plugin", "save-state", d["plugin"],
       "--params", json.dumps(d.get("params", {}), ensure_ascii=False), "-o", d["output"]]
r = subprocess.run(cmd, capture_output=True, text=True)
if r.returncode != 0:
    print("      " + r.stderr.strip().splitlines()[-1] if r.stderr.strip() else "", file=sys.stderr)
sys.exit(r.returncode)
PY
  then
    echo "state $r"
  else
    echo "FAIL  $r（生成状态文件失败，依赖它的工程会失败）"
  fi
done

ok=0; fail=0
for f in samples/${GROUP:-*}/*.muser.json; do
  [ -e "$f" ] || continue
  out="${f%.muser.json}.wav"
  if msg=$("$MUSER" render "$f" -o "$out" 2>&1); then
    ok=$((ok + 1)); echo "ok    $f"
  else
    fail=$((fail + 1)); echo "FAIL  $f"; echo "$msg" | grep -v '^\[muser\]\|^\[surge' | tail -2 | sed 's/^/      /'
  fi
done
echo "完成：成功 $ok，失败 $fail"

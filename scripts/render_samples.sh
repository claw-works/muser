#!/usr/bin/env bash
# 渲染 samples/ 下的示范工程为同名 .wav。
#
#   scripts/render_samples.sh            # 全部分组
#   scripts/render_samples.sh effects    # 只渲染某个分组
#
# 用环境变量 MUSER 指定二进制（默认 target/debug/muser）。
# plugins/ 需要 `cargo build -p muser-cli --features host` 构建的 muser；
# 依赖本机插件或音色库的工程，缺文件时会报错并跳过，不影响其它工程。
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MUSER="${MUSER:-$ROOT/target/debug/muser}"
GROUP="${1:-}"

if [ ! -x "$MUSER" ]; then
  echo "找不到 muser 二进制：$MUSER（先 cargo build -p muser-cli，或设置 MUSER）" >&2
  exit 1
fi

ok=0; fail=0
for f in "$ROOT"/samples/${GROUP:-*}/*.muser.json; do
  [ -e "$f" ] || continue
  out="${f%.muser.json}.wav"
  rel="${f#$ROOT/}"
  if msg=$("$MUSER" render "$f" -o "$out" 2>&1); then
    ok=$((ok + 1)); echo "ok    $rel"
  else
    fail=$((fail + 1)); echo "FAIL  $rel"; echo "$msg" | grep -v '^\[muser\]' | tail -2 | sed 's/^/      /'
  fi
done
echo "完成：成功 $ok，失败 $fail"

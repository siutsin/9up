#!/bin/sh
# dist writes no test block; brew audit requires one.
set -eu
file=$1
grep -q "test do" "$file" && exit 0
[ "$(tail -n 1 "$file")" = "end" ]
tmp=$(mktemp)
awk '
  { lines[NR] = $0 }
  END {
    for (i = 1; i < NR; i++) print lines[i]
    print "  test do"
    print "    assert_match version.to_s, shell_output(\"#{bin}/9up --version\")"
    print "  end"
    print lines[NR]
  }
' "$file" > "$tmp"
mv "$tmp" "$file"

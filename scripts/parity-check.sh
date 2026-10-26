#!/bin/sh
# Build one fixture tree of repositories in known states, copy it
# twice, run Python gitup on one copy and 9up on the other, and
# compare the result: every reference in every repository, the
# bookmark files, and the exit codes. Usage:
#   scripts/parity-check.sh <gitup-binary> <9up-binary>
set -eu
GITUP=$1
NINEUP=$2
"$GITUP" --version | grep -q 'gitup 0.5.2' || { echo "expected gitup 0.5.2"; exit 1; }
export GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=Parity GIT_AUTHOR_EMAIL=parity@example.com
export GIT_COMMITTER_NAME=Parity GIT_COMMITTER_EMAIL=parity@example.com
ROOT=$(mktemp -d)
trap 'rm -rf "$ROOT"' EXIT

commit_file() {
    echo "$2" > "$1/$2"
    git -C "$1" add "$2"
    git -C "$1" commit -qm "add $2"
}

SRC=$ROOT/src
mkdir -p "$SRC/subjects"
git init -q -b main --bare "$SRC/remote.git"
git clone -q "$SRC/remote.git" "$SRC/seed" 2>/dev/null
commit_file "$SRC/seed" a
git -C "$SRC/seed" push -q origin main
git -C "$SRC/seed" checkout -qb side
commit_file "$SRC/seed" s
git -C "$SRC/seed" push -q origin side
git -C "$SRC/seed" checkout -q main

for name in behind diverged no-upstream; do
    git clone -q "$SRC/remote.git" "$SRC/subjects/$name" 2>/dev/null
done
git -C "$SRC/subjects/behind" checkout -qb side origin/side
commit_file "$SRC/subjects/diverged" local-diverged
git -C "$SRC/subjects/no-upstream" checkout -qb solo
commit_file "$SRC/subjects/no-upstream" solo-file
git -C "$SRC/subjects/no-upstream" checkout -q main

git init -q -b main --bare "$SRC/subjects/bare.git"
git -C "$SRC/subjects/bare.git" remote add origin "$SRC/remote.git"
git -C "$SRC/subjects/bare.git" fetch -q origin
git -C "$SRC/subjects/bare.git" branch main refs/remotes/origin/main
git -C "$SRC/subjects/bare.git" symbolic-ref HEAD refs/heads/main

# Advance the remote after every subject was cloned.
commit_file "$SRC/seed" b
git -C "$SRC/seed" push -q origin main
git -C "$SRC/seed" checkout -q side
commit_file "$SRC/seed" s2
git -C "$SRC/seed" push -q origin side
git clone -q "$SRC/remote.git" "$SRC/subjects/ahead" 2>/dev/null
commit_file "$SRC/subjects/ahead" local-ahead

printf '# parity fixture\n%s\n' "$SRC/subjects" > "$SRC/bookmarks"

cp -a "$SRC" "$ROOT/gitup-copy"
cp -a "$SRC" "$ROOT/9up-copy"
# The two copies must point at their own remote and bookmark paths.
for copy in gitup-copy 9up-copy; do
    for name in behind ahead diverged no-upstream bare.git; do
        git -C "$ROOT/$copy/subjects/$name" remote set-url origin "$ROOT/$copy/remote.git"
    done
    printf '# parity fixture\n%s\n' "$ROOT/$copy/subjects" > "$ROOT/$copy/bookmarks"
done

export HOME=$ROOT
set +e
"$GITUP" -b "$ROOT/gitup-copy/bookmarks" > "$ROOT/gitup.out" 2>&1
GITUP_CODE=$?
"$NINEUP" -b "$ROOT/9up-copy/bookmarks" > "$ROOT/9up.out" 2>&1
NINEUP_CODE=$?
set -e
echo "exit codes: gitup=$GITUP_CODE 9up=$NINEUP_CODE"
[ "$GITUP_CODE" = "$NINEUP_CODE" ] || { echo "MISMATCH: exit codes"; exit 1; }

# The copies live at different paths, so compare with the copy
# directory normalised away.
sed "s|$ROOT/gitup-copy|COPY|g" "$ROOT/gitup-copy/bookmarks" > "$ROOT/bookmarks.norm"
sed "s|$ROOT/9up-copy|COPY|g" "$ROOT/9up-copy/bookmarks" | cmp - "$ROOT/bookmarks.norm" \
    && echo "bookmark files identical" \
    || { echo "MISMATCH: bookmark files"; exit 1; }

FAIL=0
for name in behind ahead diverged no-upstream bare.git; do
    for copy in gitup-copy 9up-copy; do
        if [ "$name" = "bare.git" ]; then
            # One known difference, recorded in the plan's Differences
            # table: gitup leaves a bare repository's HEAD branch
            # stale; 9up updates it. Exclude it here; check it below.
            git -C "$ROOT/$copy/subjects/$name" for-each-ref --format='%(refname) %(objectname)' \
                | grep -v '^refs/heads/main ' > "$ROOT/$copy.$name.refs"
        else
            git -C "$ROOT/$copy/subjects/$name" for-each-ref --format='%(refname) %(objectname)' \
                > "$ROOT/$copy.$name.refs"
        fi
        echo "HEAD $(git -C "$ROOT/$copy/subjects/$name" symbolic-ref HEAD)" >> "$ROOT/$copy.$name.refs"
    done
    if [ "$name" = "bare.git" ]; then
        NINE_MAIN=$(git -C "$ROOT/9up-copy/subjects/bare.git" rev-parse main)
        NINE_ORIGIN=$(git -C "$ROOT/9up-copy/subjects/bare.git" rev-parse refs/remotes/origin/main)
        [ "$NINE_MAIN" = "$NINE_ORIGIN" ] || { echo "MISMATCH: 9up did not update the bare HEAD branch"; FAIL=1; }
        GITUP_MAIN=$(git -C "$ROOT/gitup-copy/subjects/bare.git" rev-parse main)
        GITUP_ORIGIN=$(git -C "$ROOT/gitup-copy/subjects/bare.git" rev-parse refs/remotes/origin/main)
        [ "$GITUP_MAIN" != "$GITUP_ORIGIN" ] || { echo "MISMATCH: gitup updated the bare HEAD branch; the documented difference is gone"; FAIL=1; }
    fi
    if cmp -s "$ROOT/gitup-copy.$name.refs" "$ROOT/9up-copy.$name.refs"; then
        echo "$name: references identical"
    else
        echo "MISMATCH: $name references differ"
        diff "$ROOT/gitup-copy.$name.refs" "$ROOT/9up-copy.$name.refs" || true
        FAIL=1
    fi
done
[ "$FAIL" = 0 ] && echo "PARITY OK" || { echo "PARITY FAILED"; exit 1; }

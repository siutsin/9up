#!/usr/bin/env python3
"""Build one fixture tree of repositories in known states, copy it
twice, run Python gitup on one copy and 9up on the other, and compare
the result: every reference and HEAD in every repository, the bookmark
files, and the exit codes.

Usage: scripts/parity-check.py <gitup-binary> <9up-binary>
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SUBJECTS = ["behind", "ahead", "diverged", "no-upstream", "bare.git"]

ENV = {
    **os.environ,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_AUTHOR_NAME": "Parity",
    "GIT_AUTHOR_EMAIL": "parity@example.com",
    "GIT_COMMITTER_NAME": "Parity",
    "GIT_COMMITTER_EMAIL": "parity@example.com",
}


def run(*argv: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, env=ENV, text=True, capture_output=True, check=True)


def git(repo: Path, *args: str) -> str:
    return run("git", "-C", str(repo), *args).stdout.strip()


def commit_file(repo: Path, name: str) -> None:
    (repo / name).write_text(f"{name}\n")
    git(repo, "add", name)
    git(repo, "commit", "-qm", f"add {name}")


def build_fixture(src: Path) -> None:
    subjects = src / "subjects"
    subjects.mkdir(parents=True)
    remote = src / "remote.git"
    run("git", "init", "-q", "-b", "main", "--bare", str(remote))

    seed = src / "seed"
    run("git", "clone", "-q", str(remote), str(seed))
    commit_file(seed, "a")
    git(seed, "push", "-q", "origin", "main")
    git(seed, "checkout", "-qb", "side")
    commit_file(seed, "s")
    git(seed, "push", "-q", "origin", "side")
    git(seed, "checkout", "-q", "main")

    for name in ["behind", "diverged", "no-upstream"]:
        run("git", "clone", "-q", str(remote), str(subjects / name))
    git(subjects / "behind", "checkout", "-qb", "side", "origin/side")
    commit_file(subjects / "diverged", "local-diverged")
    git(subjects / "no-upstream", "checkout", "-qb", "solo")
    commit_file(subjects / "no-upstream", "solo-file")
    git(subjects / "no-upstream", "checkout", "-q", "main")

    bare = subjects / "bare.git"
    run("git", "init", "-q", "-b", "main", "--bare", str(bare))
    git(bare, "remote", "add", "origin", str(remote))
    git(bare, "fetch", "-q", "origin")
    git(bare, "branch", "main", "refs/remotes/origin/main")
    git(bare, "symbolic-ref", "HEAD", "refs/heads/main")

    # Advance the remote after every subject was cloned.
    commit_file(seed, "b")
    git(seed, "push", "-q", "origin", "main")
    git(seed, "checkout", "-q", "side")
    commit_file(seed, "s2")
    git(seed, "push", "-q", "origin", "side")

    # Cloned after the advance, then committed locally: purely ahead.
    run("git", "clone", "-q", str(remote), str(subjects / "ahead"))
    commit_file(subjects / "ahead", "local-ahead")


def prepare_copy(src: Path, dest: Path) -> None:
    shutil.copytree(src, dest, symlinks=True)
    # The copy must point at its own remote and bookmark paths.
    for name in SUBJECTS:
        git(dest / "subjects" / name, "remote", "set-url", "origin", str(dest / "remote.git"))
    (dest / "bookmarks").write_text(f"# parity fixture\n{dest / 'subjects'}\n")


def references(repo: Path, skip_bare_main: bool) -> list[str]:
    lines = git(repo, "for-each-ref", "--format=%(refname) %(objectname)").splitlines()
    if skip_bare_main:
        lines = [line for line in lines if not line.startswith("refs/heads/main ")]
    lines.append(f"HEAD {git(repo, 'symbolic-ref', 'HEAD')}")
    return lines


def main() -> int:
    gitup, nineup = sys.argv[1], sys.argv[2]
    version = run(gitup, "--version").stdout
    if "gitup 0.5.2" not in version:
        print("expected gitup 0.5.2")
        return 1

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        build_fixture(root / "src")
        prepare_copy(root / "src", root / "gitup-copy")
        prepare_copy(root / "src", root / "9up-copy")

        env = {**ENV, "HOME": str(root)}

        def tool(binary: str, copy: str) -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                [binary, "-b", str(root / copy / "bookmarks")],
                env=env, text=True, capture_output=True,
            )

        gitup_run = tool(gitup, "gitup-copy")
        nineup_run = tool(nineup, "9up-copy")
        print(f"exit codes: gitup={gitup_run.returncode} 9up={nineup_run.returncode}")
        if gitup_run.returncode != nineup_run.returncode:
            print("MISMATCH: exit codes")
            return 1

        # The copies live at different paths, so compare with the copy
        # directory normalised away.
        def normalised(copy: str) -> str:
            text = (root / copy / "bookmarks").read_text()
            return text.replace(str(root / copy), "COPY")

        if normalised("gitup-copy") != normalised("9up-copy"):
            print("MISMATCH: bookmark files")
            return 1
        print("bookmark files identical")

        failed = False
        for name in SUBJECTS:
            bare = name == "bare.git"
            gitup_refs = references(root / "gitup-copy" / "subjects" / name, bare)
            nineup_refs = references(root / "9up-copy" / "subjects" / name, bare)
            if gitup_refs == nineup_refs:
                print(f"{name}: references identical")
            else:
                print(f"MISMATCH: {name} references differ")
                for line in gitup_refs:
                    if line not in nineup_refs:
                        print(f"  gitup only: {line}")
                for line in nineup_refs:
                    if line not in gitup_refs:
                        print(f"  9up only:   {line}")
                failed = True
            if bare:
                # One known difference, recorded in the plan's
                # Differences table: gitup leaves a bare repository's
                # HEAD branch stale; 9up updates it. Both sides pinned.
                def bare_shas(copy: str) -> list[str]:
                    repo = root / copy / "subjects" / "bare.git"
                    return git(repo, "rev-parse", "main", "refs/remotes/origin/main").splitlines()

                if bare_shas("9up-copy")[0] != bare_shas("9up-copy")[1]:
                    print("MISMATCH: 9up did not update the bare HEAD branch")
                    failed = True
                if bare_shas("gitup-copy")[0] == bare_shas("gitup-copy")[1]:
                    print("MISMATCH: gitup updated the bare HEAD branch; the documented difference is gone")
                    failed = True

        print("PARITY FAILED" if failed else "PARITY OK")
        return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

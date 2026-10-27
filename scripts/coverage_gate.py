#!/usr/bin/env python3
"""The production coverage gate.

`cargo llvm-cov --fail-under-lines` cannot express this project's
rule: its summary counts the inline test modules' own lines, and it
merges the unit and integration records per function by taking the
largest covered count and the largest line total independently, so a
function covered jointly by both suites reads as partly missed.

This gate instead requires, for every function outside the
`#[cfg(test)]` modules, one record (the unit run or the integration
children) that covers all of its lines. It reads the same profdata
the plain report uses: line counts from `llvm-cov report
--show-functions`, and each function's identity — its first region's
position, the key llvm-cov itself groups records by — from
`llvm-cov export`.
"""

import filecmp
import glob
import json
import os
import re
import subprocess
import sys

ROW = re.compile(r"(_R\S+)\s+\d+\s+\d+\s+[\d.]+%\s+(\d+)\s+(\d+)\s+")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COV_DIR = os.path.join(ROOT, "target", "llvm-cov-target")


def run(*args):
    return subprocess.run(args, capture_output=True, text=True, check=True).stdout


def llvm_cov():
    sysroot = run("rustc", "--print", "sysroot").strip()
    host = re.search(r"^host: (\S+)", run("rustc", "-vV"), re.M).group(1)
    return os.path.join(sysroot, "lib", "rustlib", host, "bin", "llvm-cov")


def path_name(mangled):
    """The readable `module::function` path out of a v0 symbol."""
    marker = re.search(r"_\d+_9up", mangled)
    if not marker:
        return mangled
    rest, out, i = mangled[marker.end():], [], 0
    while i < len(rest) and rest[i].isdigit():
        j = i
        while j < len(rest) and rest[j].isdigit():
            j += 1
        size, k = int(rest[i:j]), j
        if k < len(rest) and rest[k] == "_" and k + 1 < len(rest) and rest[k + 1].isdigit():
            out.append(rest[k + 1:k + 1 + size])
            i = k + 1 + size
        else:
            out.append(rest[k:k + size])
            i = k + size
    return "::".join(out) or mangled


def test_module_line(path):
    """The line a source file's `#[cfg(test)]` module starts on.

    Every source file keeps its production code above the test
    module; anything at or below this line counts as test code.
    """
    with open(path) as handle:
        for number, line in enumerate(handle, 1):
            if "#[cfg(test)]" in line:
                return number
    return sys.maxsize


def records(cov, profdata, obj, sources):
    """(file, line, column) -> [(lines, covered, name)] for one object."""
    exported = json.loads(run(cov, "export", f"-instr-profile={profdata}", "-object", obj))
    identity = {}
    for function in exported["data"][0]["functions"]:
        regions = [r for r in function["regions"] if r[6] == 0]
        if not regions:
            continue
        first = min(regions, key=lambda r: (r[0], r[1]))
        identity[function["name"]] = (function["filenames"][0], first[0], first[1])
    found = {}
    for source in sources:
        out = run(cov, "report", f"-instr-profile={profdata}", obj,
                  "--show-functions", source)
        for line in out.splitlines():
            row = ROW.match(line)
            if not row or row.group(1) not in identity:
                continue
            lines, missed = int(row.group(2)), int(row.group(3))
            found.setdefault(identity[row.group(1)], []).append(
                (lines, lines - missed, row.group(1)))
    return found


def binary_name():
    """The [[bin]] name from Cargo.toml: object files are named after it."""
    with open(os.path.join(ROOT, "Cargo.toml")) as handle:
        text = handle.read()
    section = re.search(r"\[\[bin\]\](.*?)(?=\n\[|\Z)", text, re.S)
    return re.search(r'name\s*=\s*"([^"]+)"', section.group(1)).group(1)


def main():
    profdata = max(glob.glob(os.path.join(COV_DIR, "*.profdata")), key=os.path.getmtime)
    stem = binary_name()
    shipped = os.path.join(COV_DIR, "debug", stem)
    cov = llvm_cov()
    sources = sorted(glob.glob(os.path.join(ROOT, "src", "*.rs")))
    # The unit harness is the deps binary that is not a byte-identical
    # copy of the shipped binary and whose functions include the test
    # modules. (A substring search is not enough: the report header
    # prints the source path, which can itself contain "tests".)
    harness = None
    for candidate in glob.glob(os.path.join(COV_DIR, "debug", "deps", stem + "-*")):
        if "." in os.path.basename(candidate) or filecmp.cmp(candidate, shipped, shallow=False):
            continue
        out = run(cov, "report", f"-instr-profile={profdata}", candidate,
                  "--show-functions", sources[0])
        names = [path_name(row.group(1)) for row in map(ROW.match, out.splitlines()) if row]
        if any("::tests::" in name for name in names):
            harness = candidate
            break
    if harness is None:
        sys.exit("coverage gate: the unit test harness was not found")
    groups = {}
    for obj in (shipped, harness):
        for key, recs in records(cov, profdata, obj, sources).items():
            groups.setdefault(key, []).extend(recs)
    failures = []
    total = 0
    for (filename, line, _column), recs in sorted(groups.items()):
        relative = os.path.relpath(filename, ROOT)
        if not relative.startswith("src/"):
            continue
        source = os.path.join(ROOT, relative)
        if line >= test_module_line(source):
            continue
        total += 1
        deficit = max(r[0] for r in recs) - max(r[1] for r in recs)
        if deficit > 0:
            failures.append(f"{relative}:{line} {path_name(recs[0][2])}: "
                            f"{deficit} line(s) covered by no single test run")
    if failures:
        print("coverage gate: production functions not fully covered:")
        print("\n".join(failures))
        return 1
    print(f"coverage gate: all {total} production functions fully covered")
    return 0


if __name__ == "__main__":
    sys.exit(main())

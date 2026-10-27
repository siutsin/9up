//! The production coverage gate.
//!
//! `cargo llvm-cov --fail-under-lines` cannot express this project's
//! rule: its summary counts the inline test modules' own lines, and it
//! merges the unit and integration records per function by taking the
//! largest covered count and the largest line total independently, so
//! a function covered jointly by both suites reads as partly missed.
//!
//! This gate instead requires, for every function outside the
//! `#[cfg(test)]` modules, one record (the unit run or the integration
//! children) that covers all of its lines. It reads the same profdata
//! the plain report uses: line counts from `llvm-cov report
//! --show-functions`, and each function's identity — its first
//! region's position, the key llvm-cov itself groups records by —
//! from `llvm-cov export`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// (file, first-region line, first-region column)
type Key = (PathBuf, u64, u64);

/// Function identity -> (lines, covered, mangled name) per record.
type Records = HashMap<Key, Vec<(u64, u64, String)>>;

enum Gate {
    Passed(usize),
    Failed(Vec<String>),
}

fn main() -> ExitCode {
    match gate() {
        Ok(Gate::Passed(total)) => {
            println!("coverage gate: all {total} production functions fully covered");
            ExitCode::SUCCESS
        }
        Ok(Gate::Failed(failures)) => {
            println!("coverage gate: production functions not fully covered:");
            for failure in failures {
                println!("{failure}");
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("coverage gate: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(program: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    if !output.status.success() {
        return Err(format!("{} failed: {}", program.display(), output.status));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("non-UTF-8 output: {e}"))
}

fn root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "the xtask directory has no parent".to_string())
}

fn llvm_cov() -> Result<PathBuf, String> {
    let rustc = Path::new("rustc");
    let sysroot = run(rustc, &["--print", "sysroot"])?;
    let version = run(rustc, &["-vV"])?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc -vV names no host")?;
    let path = Path::new(sysroot.trim())
        .join("lib/rustlib")
        .join(host)
        .join("bin/llvm-cov");
    if path.exists() {
        Ok(path)
    } else {
        Ok(path.with_extension("exe"))
    }
}

/// The readable `module::function` path out of a v0 symbol.
fn path_name(mangled: &str) -> String {
    let bytes = mangled.as_bytes();
    let mut rest = None;
    for i in 0..bytes.len() {
        if bytes[i] != b'_' {
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 && mangled[j..].starts_with("_9up") {
            rest = Some(&mangled[j + 4..]);
            break;
        }
    }
    let Some(rest) = rest else {
        return mangled.to_string();
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        let Ok(size) = rest[i..j].parse::<usize>() else {
            break;
        };
        if j < bytes.len()
            && bytes[j] == b'_'
            && j + 1 < bytes.len()
            && bytes[j + 1].is_ascii_digit()
        {
            if j + 1 + size > bytes.len() {
                break;
            }
            out.push(&rest[j + 1..j + 1 + size]);
            i = j + 1 + size;
        } else {
            if j + size > bytes.len() {
                break;
            }
            out.push(&rest[j..j + size]);
            i = j + size;
        }
    }
    if out.is_empty() {
        mangled.to_string()
    } else {
        out.join("::")
    }
}

/// The line a source file's `#[cfg(test)]` module starts on.
///
/// Every source file keeps its production code above the test
/// module; anything at or below this line counts as test code.
fn test_module_line(path: &Path) -> Result<usize, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(text
        .lines()
        .position(|line| line.contains("#[cfg(test)]"))
        .map_or(usize::MAX, |index| index + 1))
}

/// (lines, covered, mangled name) per function identity in one object.
fn records(
    cov: &Path,
    profdata: &Path,
    obj: &Path,
    sources: &[PathBuf],
) -> Result<Records, String> {
    let profile = format!("-instr-profile={}", profdata.display());
    let exported = run(
        cov,
        &["export", &profile, "-object", &obj.to_string_lossy()],
    )?;
    let json: serde_json::Value =
        serde_json::from_str(&exported).map_err(|e| format!("bad export JSON: {e}"))?;
    let mut identity: HashMap<String, Key> = HashMap::new();
    let functions = json["data"][0]["functions"]
        .as_array()
        .ok_or("the export has no functions")?;
    for function in functions {
        let first = function["regions"]
            .as_array()
            .ok_or("a function has no regions")?
            .iter()
            .filter(|region| region[6].as_u64() == Some(0))
            .filter_map(|region| Some((region[0].as_u64()?, region[1].as_u64()?)))
            .min();
        let (Some(name), Some(file), Some((line, column))) = (
            function["name"].as_str(),
            function["filenames"][0].as_str(),
            first,
        ) else {
            continue;
        };
        identity.insert(name.to_string(), (PathBuf::from(file), line, column));
    }
    let mut found: Records = HashMap::new();
    for source in sources {
        let out = run(
            cov,
            &[
                "report",
                &profile,
                &obj.to_string_lossy(),
                "--show-functions",
                &source.to_string_lossy(),
            ],
        )?;
        for line in out.lines() {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < 6 || !tokens[0].starts_with("_R") {
                continue;
            }
            let Some(key) = identity.get(tokens[0]) else {
                continue;
            };
            let (Ok(lines), Ok(missed)) = (tokens[4].parse::<u64>(), tokens[5].parse::<u64>())
            else {
                continue;
            };
            found.entry(key.clone()).or_default().push((
                lines,
                lines.saturating_sub(missed),
                tokens[0].to_string(),
            ));
        }
    }
    Ok(found)
}

/// The `[[bin]]` name from Cargo.toml: object files are named after it.
fn binary_name(root: &Path) -> Result<String, String> {
    let text = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|e| format!("cannot read Cargo.toml: {e}"))?;
    let section = text
        .split("[[bin]]")
        .nth(1)
        .ok_or("Cargo.toml has no [[bin]] section")?;
    let section = section.split("\n[").next().unwrap_or(section);
    let name = section
        .split("name = \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .ok_or("the [[bin]] section names no binary")?;
    Ok(name.to_string())
}

fn gate() -> Result<Gate, String> {
    let root = root()?;
    let cov_dir = root.join("target/llvm-cov-target");
    let mut profdata: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in
        fs::read_dir(&cov_dir).map_err(|e| format!("cannot list {}: {e}", cov_dir.display()))?
    {
        let entry = entry.map_err(|e| format!("cannot read a directory entry: {e}"))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "profdata") {
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
            if profdata.as_ref().is_none_or(|(best, _)| modified > *best) {
                profdata = Some((modified, path));
            }
        }
    }
    let profdata = profdata.map(|(_, path)| path).ok_or("no profdata found")?;
    let stem = binary_name(&root)?;
    let shipped = cov_dir.join("debug").join(&stem);
    let cov = llvm_cov()?;
    let mut sources: Vec<PathBuf> = fs::read_dir(root.join("src"))
        .map_err(|e| format!("cannot list src: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    sources.sort();
    // The unit harness is the deps binary that is not a byte-identical
    // copy of the shipped binary and whose functions include the test
    // modules. (A substring search is not enough: the report header
    // prints the source path, which can itself contain "tests".)
    let shipped_bytes =
        fs::read(&shipped).map_err(|e| format!("cannot read {}: {e}", shipped.display()))?;
    let profile = format!("-instr-profile={}", profdata.display());
    let mut harness = None;
    let deps = cov_dir.join("debug/deps");
    for entry in fs::read_dir(&deps).map_err(|e| format!("cannot list {}: {e}", deps.display()))? {
        let entry = entry.map_err(|e| format!("cannot read a directory entry: {e}"))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&format!("{stem}-")) || name.contains('.') {
            continue;
        }
        if fs::read(&path).is_ok_and(|bytes| bytes == shipped_bytes) {
            continue;
        }
        let out = run(
            &cov,
            &[
                "report",
                &profile,
                &path.to_string_lossy(),
                "--show-functions",
                &sources[0].to_string_lossy(),
            ],
        )?;
        let has_tests = out.lines().any(|line| {
            line.split_whitespace().next().is_some_and(|token| {
                token.starts_with("_R") && path_name(token).contains("::tests::")
            })
        });
        if has_tests {
            harness = Some(path);
            break;
        }
    }
    let harness = harness.ok_or("the unit test harness was not found")?;
    let mut groups: Records = HashMap::new();
    for obj in [&shipped, &harness] {
        for (key, recs) in records(&cov, &profdata, obj, &sources)? {
            groups.entry(key).or_default().extend(recs);
        }
    }
    let mut keys: Vec<&Key> = groups.keys().collect();
    keys.sort();
    let mut module_lines: HashMap<PathBuf, usize> = HashMap::new();
    let mut failures = Vec::new();
    let mut total = 0;
    for key in keys {
        let (filename, line, _) = key;
        let Ok(relative) = filename.strip_prefix(&root) else {
            continue;
        };
        if !relative.starts_with("src") {
            continue;
        }
        let boundary = match module_lines.get(filename) {
            Some(boundary) => *boundary,
            None => {
                let boundary = test_module_line(filename)?;
                module_lines.insert(filename.clone(), boundary);
                boundary
            }
        };
        if (*line as usize) >= boundary {
            continue;
        }
        total += 1;
        let recs = &groups[key];
        let deficit = recs.iter().map(|rec| rec.0).max().unwrap_or(0)
            - recs.iter().map(|rec| rec.1).max().unwrap_or(0);
        if deficit > 0 {
            failures.push(format!(
                "{}:{} {}: {} line(s) covered by no single test run",
                relative.display(),
                line,
                path_name(&recs[0].2),
                deficit
            ));
        }
    }
    if failures.is_empty() {
        Ok(Gate::Passed(total))
    } else {
        Ok(Gate::Failed(failures))
    }
}

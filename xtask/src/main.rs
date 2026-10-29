use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

/// One failing function: where its first region sits, and its raw symbol name.
struct Failure {
    file: String,
    line: u64,
    col: u64,
    name: String,
}

/// The production sources this gate judges: the root package's `src`, one level up from here.
fn production_dir() -> Option<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("src"))
}

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: xtask <coverage.json>");
        return ExitCode::from(2);
    };
    let Some(src_dir) = production_dir() else {
        eprintln!("xtask: cannot locate the workspace root");
        return ExitCode::from(2);
    };
    let export: Value = match fs::read_to_string(&path)
        .map_err(|err| format!("cannot read {path}: {err}"))
        .and_then(|text| {
            serde_json::from_str(&text).map_err(|err| format!("cannot parse {path}: {err}"))
        }) {
        Ok(export) => export,
        Err(message) => {
            eprintln!("xtask: {message}");
            return ExitCode::from(2);
        }
    };
    match check(&export, &src_dir) {
        Ok((checked, failures)) if failures.is_empty() => {
            println!("{checked} production functions checked: all fully covered.");
            ExitCode::SUCCESS
        }
        Ok((checked, failures)) => {
            for failure in &failures {
                println!(
                    "{}:{}:{}: {}",
                    failure.file, failure.line, failure.col, failure.name
                );
            }
            println!(
                "{} of {checked} production functions are not fully covered.",
                failures.len()
            );
            ExitCode::FAILURE
        }
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::from(2)
        }
    }
}

/// Apply the gate to one export: group records by their function's first region, then a
/// group passes when one record has every code region executed. Returns how many
/// production functions were checked, and one failure per group no record covers in full.
fn check(export: &Value, src_dir: &Path) -> Result<(usize, Vec<Failure>), String> {
    let functions = export
        .get("data")
        .and_then(Value::as_array)
        .and_then(|data| data.first())
        .and_then(|data| data.get("functions"))
        .and_then(Value::as_array)
        .ok_or("the export has no function records")?;
    // One source function has one record per binary that compiled it (the unit-test
    // harness and the shipped binary); the records share the first region's location.
    let mut groups: BTreeMap<(String, u64, u64), (String, bool)> = BTreeMap::new();
    for record in functions {
        let Some(regions) = record.get("regions").and_then(Value::as_array) else {
            continue;
        };
        let Some(first) = regions.first() else {
            continue;
        };
        let Some(filenames) = record.get("filenames").and_then(Value::as_array) else {
            continue;
        };
        let (Some(line), Some(col), Some(file_id)) = (at(first, 0), at(first, 1), at(first, 5))
        else {
            continue;
        };
        let Some(file) = filenames
            .get(usize::try_from(file_id).unwrap_or(usize::MAX))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !is_production(src_dir, file) {
            continue;
        }
        // Region fields: [start line, start column, end line, end column, count,
        // file id, expanded file id, kind]. Kind 0 is a code region; only those count.
        let covered = regions
            .iter()
            .all(|region| at(region, 7) != Some(0) || at(region, 4).is_some_and(|count| count > 0));
        let name = record
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let entry = groups
            .entry((file.to_owned(), line, col))
            .or_insert_with(|| (name.to_owned(), false));
        entry.1 |= covered;
    }
    // An export matching nothing in this tree means the data pipeline broke; a green
    // run that checked nothing is the failure this gate exists to prevent.
    if groups.is_empty() {
        return Err("the export holds no production functions".to_owned());
    }
    let failures = groups
        .iter()
        .filter(|(_, (_, passed))| !passed)
        .map(|((file, line, col), (name, _))| Failure {
            file: file.clone(),
            line: *line,
            col: *col,
            name: name.clone(),
        })
        .collect();
    Ok((groups.len(), failures))
}

/// One numeric field of a region, by position.
fn at(region: &Value, index: usize) -> Option<u64> {
    region.get(index).and_then(Value::as_u64)
}

/// Production means under the crate's `src` and not a test file; the match is on the
/// file name alone, never a substring, so a path containing "tests" stays production.
fn is_production(src_dir: &Path, file: &str) -> bool {
    let path = Path::new(file);
    path.starts_with(src_dir) && path.file_name().is_some_and(|name| name != "tests.rs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(name: &str, file: &str, regions: Value) -> Value {
        json!({"name": name, "filenames": [file], "regions": regions})
    }

    fn export(functions: Value) -> Value {
        json!({"data": [{"functions": functions}]})
    }

    fn src() -> PathBuf {
        PathBuf::from("/repo/src")
    }

    /// Guards the one-record rule: a second binary's full record rescues a partial one.
    #[test]
    fn one_full_record_covers_the_function() {
        let export = export(json!([
            record(
                "f_unit",
                "/repo/src/lib.rs",
                json!([[10, 1, 10, 5, 1, 0, 0, 0], [11, 1, 11, 5, 0, 0, 0, 0]])
            ),
            record(
                "f_bin",
                "/repo/src/lib.rs",
                json!([[10, 1, 10, 5, 3, 0, 0, 0], [11, 1, 11, 5, 2, 0, 0, 0]])
            ),
        ]));
        let (checked, failures) = check(&export, &src()).unwrap();
        assert_eq!(checked, 1);
        assert!(failures.is_empty());
    }

    /// Guards the failure report: file, line and symbol name of the uncovered function.
    #[test]
    fn a_zero_code_region_fails_and_names_the_function() {
        let export = export(json!([record(
            "some_symbol",
            "/repo/src/lib.rs",
            json!([[10, 1, 10, 5, 1, 0, 0, 0], [12, 3, 12, 9, 0, 0, 0, 0]])
        ),]));
        let (checked, failures) = check(&export, &src()).unwrap();
        assert_eq!(checked, 1);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].file, "/repo/src/lib.rs");
        assert_eq!(failures[0].line, 10);
        assert_eq!(failures[0].name, "some_symbol");
    }

    /// Guards that only code regions gate: an untaken branch region does not fail a function.
    #[test]
    fn a_zero_non_code_region_does_not_fail() {
        let export = export(json!([record(
            "f",
            "/repo/src/lib.rs",
            json!([[10, 1, 10, 5, 1, 0, 0, 0], [10, 6, 10, 9, 0, 0, 0, 3]])
        ),]));
        let (checked, failures) = check(&export, &src()).unwrap();
        assert_eq!(checked, 1);
        assert!(failures.is_empty());
    }

    /// Guards the production filter: test files, tooling and integration tests never gate.
    #[test]
    fn test_files_and_foreign_files_are_not_production() {
        let export = export(json!([
            record(
                "a",
                "/repo/src/config/tests.rs",
                json!([[1, 1, 1, 5, 0, 0, 0, 0]])
            ),
            record(
                "b",
                "/repo/xtask/src/main.rs",
                json!([[1, 1, 1, 5, 0, 0, 0, 0]])
            ),
            record("c", "/repo/tests/cli.rs", json!([[1, 1, 1, 5, 0, 0, 0, 0]])),
        ]));
        assert!(check(&export, &src()).is_err());
    }

    /// Guards the empty gate: an export with no production function in it fails the
    /// run, so a broken data pipeline cannot pass by checking nothing.
    #[test]
    fn a_record_without_regions_is_skipped() {
        let export = export(json!([record("f", "/repo/src/lib.rs", json!([]))]));
        assert!(check(&export, &src()).is_err());
    }

    /// Guards the grouping key: the same line and column in two files is two functions.
    #[test]
    fn the_same_location_in_two_files_is_two_functions() {
        let export = export(json!([
            record("a", "/repo/src/a.rs", json!([[10, 1, 10, 5, 1, 0, 0, 0]])),
            record("b", "/repo/src/b.rs", json!([[10, 1, 10, 5, 0, 0, 0, 0]])),
        ]));
        let (checked, failures) = check(&export, &src()).unwrap();
        assert_eq!(checked, 2);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].name, "b");
    }
}

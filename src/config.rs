use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Value};

use crate::sys::Sys;

/// The config file: a non-empty `-C` path wins, else `9up/config.toml` under the config
/// directory, or under a relative `.config` when there is no home directory.
pub fn file_path(flag: Option<&str>, config_dir: Option<&Path>) -> PathBuf {
    if let Some(path) = flag.filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    // `config_dir` comes from `main`: an absolute `XDG_CONFIG_HOME`, else `~/.config`.
    let config = config_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".config"));
    config.join("9up").join("config.toml")
}

/// The config file's entries, in file order. A missing or unreadable file means no repositories.
/// Invalid UTF-8, invalid TOML, or a `repositories` value that is not an array of strings fails:
/// the error prints to standard error and the run exits 1.
pub fn load(sys: &dyn Sys, path: &Path) -> io::Result<Vec<String>> {
    let Ok(bytes) = sys.read(path) else {
        return Ok(Vec::new());
    };
    entries(&parse(path, bytes)?, path)
}

/// Edit the file's document in place: only `repositories` changes; comments and other keys stay.
/// A missing file is created with that one key, one entry per line.
pub fn save(sys: &dyn Sys, path: &Path, entries: &[String]) -> io::Result<()> {
    let mut doc = match sys.read(path) {
        Ok(bytes) => parse(path, bytes)?,
        // Only a missing file starts fresh. Any other read failure is an error:
        // overwriting would drop the file's other keys unseen.
        Err(err) if err.kind() == io::ErrorKind::NotFound => DocumentMut::new(),
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!("cannot read {}: {err}", path.display()),
            ));
        }
    };
    match doc.get_mut("repositories").and_then(Item::as_array_mut) {
        Some(array) => sync_array(array, entries),
        None => doc["repositories"] = Item::Value(Value::Array(repositories_array(entries))),
    }
    let mut text = doc.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    sys.write(path, text.as_bytes()).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("cannot write {}: {err}", path.display()),
        )
    })
}

/// Update the array in place, not rebuilt: drop values the list lost, append values it gained.
/// Kept values keep their formatting and comments.
fn sync_array(array: &mut Array, entries: &[String]) {
    // Keep the longest prefix of the list found in the array in order; drop the rest of the
    // array and append the rest of the list. In-order matching keeps duplicates straight:
    // with ["/a", "/b", "/a"] and a list of ["/b", "/a"], the first "/a" goes.
    let mut keep = vec![false; array.len()];
    let mut scan = 0;
    let mut kept = 0;
    for entry in entries {
        let Some(found) = (scan..array.len())
            .find(|&index| array.get(index).and_then(Value::as_str) == Some(entry.as_str()))
        else {
            break;
        };
        keep[found] = true;
        scan = found + 1;
        kept += 1;
    }
    let needed = &entries[kept..];
    let mut index = 0;
    while index < array.len() {
        if keep[index] {
            index += 1;
        } else {
            array.remove(index);
            keep.remove(index);
        }
    }
    // Removing the last value can take the break before `]` with it, gluing `]` onto the last
    // value's line, where a comment would comment it out. A multiline array gets the break back.
    let trailing = array.trailing().as_str().unwrap_or_default();
    let multiline =
        trailing.contains('\n') || array.iter().any(|value| prefix_of(value).contains('\n'));
    if !trailing.ends_with('\n') && (trailing.contains('#') || multiline) {
        array.set_trailing(format!("{trailing}\n"));
    }
    if needed.is_empty() {
        return;
    }
    // Without a trailing comma, the last value's suffix holds the break before `]`. Left
    // there, an appended value's comma renders after that break, on a line of its own.
    if !array.is_empty() {
        let suffix = array
            .iter()
            .last()
            .map(suffix_of)
            .unwrap_or_default()
            .to_owned();
        if !suffix.contains('#') && !suffix.is_empty() {
            let trailing_empty = array.trailing().as_str().unwrap_or_default().is_empty();
            let mut last = array.remove(array.len() - 1);
            last.decor_mut().set_suffix("");
            array.push_formatted(last);
            if trailing_empty {
                array.set_trailing(suffix);
            }
        }
    }
    let was_empty = array.is_empty();
    // A comment after the last value, on its line, sits in the array's trailing text.
    // The first new value takes it as a prefix, so the comment stays on that line.
    let trailing = array.trailing().as_str().unwrap_or_default().to_owned();
    let mut first_prefix = None;
    if !trailing.starts_with('\n')
        && let Some(comment) = line_comment(&trailing)
    {
        let rest = trailing[comment.len()..].to_owned();
        array.set_trailing(rest);
        first_prefix = Some(format!("{comment}{}", append_prefix(array)));
    }
    let prefix = append_prefix(array);
    for entry in needed {
        let decor = first_prefix.take().unwrap_or_else(|| prefix.clone());
        array.push_formatted(Value::from(entry).decorated(decor, ""));
    }
    if was_empty {
        // The trailing text of an empty array can hold a comment.
        // Only a comment-free one is replaced with the plain break.
        let trailing = array.trailing().as_str().unwrap_or_default();
        if !trailing.contains('#') {
            array.set_trailing("\n");
        }
    }
}

/// The raw text a value's decor holds before it.
fn prefix_of(value: &Value) -> &str {
    value
        .decor()
        .prefix()
        .and_then(|raw| raw.as_str())
        .unwrap_or_default()
}

/// The raw text a value's decor holds after it.
fn suffix_of(value: &Value) -> &str {
    value
        .decor()
        .suffix()
        .and_then(|raw| raw.as_str())
        .unwrap_or_default()
}

/// The first line of a raw text, when it holds a comment.
fn line_comment(text: &str) -> Option<String> {
    let line = match text.find('\n') {
        Some(index) => &text[..index],
        None => text,
    };
    line.contains('#').then(|| line.to_owned())
}

/// A new value's whitespace: the last value's line break and indentation, minus any decor comment.
/// An inline array takes a space; an empty one, the one-key document's one-entry-per-line form.
fn append_prefix(array: &Array) -> String {
    let Some(last) = array.iter().last() else {
        return "\n    ".to_owned();
    };
    let prefix = prefix_of(last);
    match prefix.rfind('\n') {
        Some(index) => prefix[index..].to_owned(),
        None => " ".to_owned(),
    }
}

/// Shared by `load` and `save`, so both reject the same bad files with the same message.
fn parse(path: &Path, bytes: Vec<u8>) -> io::Result<DocumentMut> {
    let text = String::from_utf8(bytes).map_err(|err| unparseable(path, err))?;
    text.parse::<DocumentMut>()
        .map_err(|err| unparseable(path, err))
}

/// Read `repositories` strictly, so a wrong type is an error and not an empty list.
fn entries(doc: &DocumentMut, path: &Path) -> io::Result<Vec<String>> {
    let Some(item) = doc.get("repositories") else {
        return Ok(Vec::new());
    };
    let Some(array) = item.as_array() else {
        return Err(not_strings(path));
    };
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| not_strings(path))
        })
        .collect()
}

/// Written one entry per line, so each add or delete changes a single line in version control.
fn repositories_array(entries: &[String]) -> Array {
    let mut array = Array::new();
    for entry in entries {
        array.push_formatted(Value::from(entry.as_str()).decorated("\n    ", ""));
    }
    if !entries.is_empty() {
        array.set_trailing("\n");
    }
    array
}

/// One error form for every parse failure, so the message names the file.
fn unparseable(path: &Path, err: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot parse {}: {err}", path.display()),
    )
}

/// The error for a `repositories` value of the wrong type, so the user knows what to fix.
fn not_strings(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "cannot parse {}: `repositories` is not an array of strings",
            path.display()
        ),
    )
}

/// A path's stored form: absolute, normalised without filesystem access. A leading `~` stays.
pub fn normalize(path: &str, cwd: &Path) -> String {
    if path.is_empty() {
        return cwd.to_string_lossy().into_owned();
    }
    let full = if path == "~" || path.starts_with("~/") || Path::new(path).is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path).to_string_lossy().into_owned()
    };
    let absolute = full.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in full.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                // Only a sole leading `~` is protected: a mid-path `~`
                // is an ordinary component that `..` can pop.
                if !(parts.len() == 1 && parts[0] == "~") {
                    parts.pop();
                }
            }
            part => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests;

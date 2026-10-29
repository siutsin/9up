use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Value};

use crate::sys::Sys;

/// The config file for a run: an explicit `-C` path wins; an empty or
/// missing flag means `config.toml` in a `9up` folder under the platform
/// config directory, or under `.config` when the platform has none.
pub fn file_path(flag: Option<&str>, config_dir: Option<&Path>) -> PathBuf {
    if let Some(path) = flag.filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    // The config directory comes from `dirs` (see main): on Linux a
    // relative XDG_CONFIG_HOME is ignored, as the specification
    // requires. No config directory at all falls back to `.config`.
    let config = config_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".config"));
    config.join("9up").join("config.toml")
}

/// The config file's entries, in file order.
///
/// A missing or unreadable file means no repositories. A file that is
/// not valid TOML, whose content is not valid UTF-8, or whose
/// `repositories` value is not an array of strings cannot be parsed:
/// the error prints to standard error and the run exits 1.
pub fn load(sys: &dyn Sys, path: &Path) -> io::Result<Vec<String>> {
    let Ok(bytes) = sys.read(path) else {
        return Ok(Vec::new());
    };
    entries(&parse(path, bytes)?, path)
}

/// Write the entries by editing the file's document in place, so
/// comments and any other keys stay as they are; only the
/// `repositories` array changes. A file that does not exist yet is
/// created as the one key, with each entry on its own line.
pub fn save(sys: &dyn Sys, path: &Path, entries: &[String]) -> io::Result<()> {
    let mut doc = match sys.read(path) {
        Ok(bytes) => parse(path, bytes)?,
        // Only a missing file starts a fresh document. Any other read
        // failure is an error: overwriting would drop the file's
        // other keys unseen.
        Err(err) if err.kind() == io::ErrorKind::NotFound => DocumentMut::new(),
        Err(err) => return Err(err),
    };
    match doc.get_mut("repositories").and_then(Item::as_array_mut) {
        Some(array) => sync_array(array, entries),
        None => doc["repositories"] = Item::Value(Value::Array(repositories_array(entries))),
    }
    let mut text = doc.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    sys.write(path, text.as_bytes())
}

/// Bring an existing array to the entry list without rebuilding it:
/// values the list no longer holds are removed, values it adds are
/// appended. Kept values keep their formatting and their comments.
fn sync_array(array: &mut Array, entries: &[String]) {
    // The kept values are the longest prefix of the entry list that
    // already appears in the array in order: everything else in the
    // array goes, and the rest of the list is appended. Matching in
    // order keeps duplicates straight — with ["/a", "/b", "/a"] and
    // a list of ["/b", "/a"], the first "/a" is the one removed.
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
            rescue_comments(array, index);
            array.remove(index);
            keep.remove(index);
        }
    }
    // Removing the last value can take the line break before the
    // closing bracket with it: the bracket would glue onto the last
    // value's line, and a comment there would comment it out. A
    // multiline array gets the break back.
    let trailing = array.trailing().as_str().unwrap_or_default();
    let multiline =
        trailing.contains('\n') || array.iter().any(|value| prefix_of(value).contains('\n'));
    if !trailing.ends_with('\n') && (trailing.contains('#') || multiline) {
        array.set_trailing(format!("{trailing}\n"));
    }
    if needed.is_empty() {
        return;
    }
    let was_empty = array.is_empty();
    // A comment that trails the last value on its own line sits in
    // the array's trailing text. The first new value takes it as a
    // prefix, so the comment stays on that line.
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

/// toml_edit stores the raw text around a value on the value itself:
/// a comment before it in its prefix, a comment after it (before its
/// comma) in its suffix. Both can hold a comment that must survive
/// the value's removal, so move them out first: the prefix comment
/// forward to the next value, the suffix comment back onto the
/// previous value's line when that line has no comment of its own,
/// and forward when there is no previous value.
fn rescue_comments(array: &mut Array, index: usize) {
    let (prefix, suffix) = array
        .get(index)
        .map(|value| (prefix_of(value), suffix_of(value)))
        .unwrap_or_default();
    let head = comment_head(prefix);
    let mut notes = suffix_comments(suffix).into_iter();
    let mut forward: Vec<String> = Vec::new();
    // The first suffix comment moves back onto the previous value's
    // line when that line has no comment of its own; the rest go
    // forward, in file order after the head comment.
    if let Some(first) = notes.next() {
        let mut moved_back = false;
        if index > 0
            && let Some(previous) = array.get_mut(index - 1)
        {
            let old = suffix_of(previous);
            if !old.contains('#') {
                let old = old.to_owned();
                previous.decor_mut().set_suffix(format!("{old}{first}"));
                moved_back = true;
            }
        }
        if !moved_back {
            forward.push(first);
        }
    }
    forward.extend(notes);
    if let Some(head) = &head {
        move_forward(array, index, head);
    }
    for note in &forward {
        move_forward(array, index, note);
    }
}

/// The comments inside a raw suffix, one per line that holds one,
/// each in the " # ..." form a comment takes after a value. A
/// suffix can span lines: the last value's suffix reaches the
/// closing bracket, so a comment on its own line before the
/// bracket lives there too.
fn suffix_comments(suffix: &str) -> Vec<String> {
    suffix
        .lines()
        .filter(|line| line.contains('#'))
        .map(|line| format!(" {}", line.trim_start()))
        .collect()
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

/// The part of a raw prefix before its final line break, when it
/// holds a comment. The break and the indentation after it belong
/// to the value the prefix precedes.
fn comment_head(prefix: &str) -> Option<String> {
    let head = match prefix.rfind('\n') {
        Some(index) => &prefix[..index],
        None => prefix,
    };
    head.contains('#').then(|| head.to_owned())
}

/// The first line of a raw text, when it holds a comment.
fn line_comment(text: &str) -> Option<String> {
    let line = match text.find('\n') {
        Some(index) => &text[..index],
        None => text,
    };
    line.contains('#').then(|| line.to_owned())
}

/// Move a rescued comment to the next value's prefix, or to the
/// array's trailing text when the removed value was last.
fn move_forward(array: &mut Array, index: usize, text: &str) {
    if let Some(next) = array.get_mut(index + 1) {
        let old = prefix_of(next).to_owned();
        next.decor_mut().set_prefix(join(text, &old));
    } else {
        let trailing = array.trailing().as_str().unwrap_or_default();
        array.set_trailing(join(text, trailing));
    }
}

/// Join a rescued comment onto the raw text that follows it. When
/// that text opens on the same line (a comment of its own), the
/// rescued comment takes its own line first, so the two comments
/// never merge into one.
fn join(head: &str, rest: &str) -> String {
    if rest.is_empty() || rest.starts_with('\n') {
        return format!("{head}{rest}");
    }
    let last_line = head.rsplit('\n').next().unwrap_or_default();
    let indent = last_line.len() - last_line.trim_start_matches(' ').len();
    let indent = if indent == 0 { 4 } else { indent };
    format!(
        "{head}\n{}{}",
        " ".repeat(indent),
        rest.trim_start_matches(' ')
    )
}

/// The whitespace a new value takes: the last value's line break and
/// indentation, with any comment in its raw decor left out. An inline
/// array takes a space. An empty array takes the one-key document's
/// one-entry-per-line form.
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

fn parse(path: &Path, bytes: Vec<u8>) -> io::Result<DocumentMut> {
    let text = String::from_utf8(bytes).map_err(|err| unparseable(path, err))?;
    text.parse::<DocumentMut>()
        .map_err(|err| unparseable(path, err))
}

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

fn unparseable(path: &Path, err: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot parse {}: {err}", path.display()),
    )
}

fn not_strings(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "cannot parse {}: `repositories` is not an array of strings",
            path.display()
        ),
    )
}

/// The stored form of a path: absolute, normalised with no filesystem
/// access. A leading `~` stays, and only the rest is normalised.
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

use std::ffi::OsString;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// App context: system services, resolved values and output, built once by `main` and passed down.
/// Nothing below `main` reads the environment or builds a service, except test stubs.
pub struct App<'a> {
    pub sys: Box<dyn Sys>,
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
    pub config_file: PathBuf,
    pub colour: bool,
    pub out: &'a mut (dyn io::Write + 'a),
}

/// OS services; tests swap in a stub to force failures a real system will not produce on cue.
pub trait Sys {
    /// Run git in `dir` with null stdin and an English locale, as branch logic reads git's words.
    /// The default fails: a test stub overrides only what it scripts.
    fn git(&self, _dir: &Path, _argv: &[&str]) -> io::Result<Output> {
        Err(io::Error::other("not implemented in this stub"))
    }

    /// Run the user's command in `dir`, capturing output; the caller may ignore the exit status.
    fn exec(&self, _dir: &Path, _argv: &[String]) -> io::Result<Output> {
        Err(io::Error::other("not implemented in this stub"))
    }

    /// Read a whole file. The default fails, so a stub not scripting reads never touches the disk.
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::other("not implemented in this stub"))
    }

    /// Write a file, creating its parent directory when it has one.
    fn write(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Err(io::Error::other("not implemented in this stub"))
    }
}

/// The real services, over the OS.
pub struct RealSys;

/// Shared spawn setup: git and the user's command never block on the terminal or run elsewhere.
fn base(dir: &Path, program: &str) -> Command {
    let mut command = Command::new(program);
    command.current_dir(dir).stdin(Stdio::null());
    command
}

/// Build the git command: the `-c` pairs come first and neutralise the command-running keys
/// they name, and inherited `GIT_*` state is scrubbed. Repo-local `credential.helper` (on an
/// https fetch), `core.gitProxy` (on a git:// fetch) and `filter.<driver>.*` (on a merge
/// checkout) are not overridden: filter drivers have no `-c` way to neutralise them, and the
/// other two stay honoured, like the user's setup.
fn git_command(
    dir: &Path,
    argv: &[&str],
    env_names: impl IntoIterator<Item = OsString>,
) -> Command {
    let mut command = base(dir, "git");
    command
        .args([
            "-c",
            "protocol.ext.allow=never",
            "-c",
            // The stock ssh, not an empty value: an empty one is a command git cannot run.
            "core.sshCommand=ssh",
            "-c",
            "core.fsmonitor=",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(argv)
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_SSH_COMMAND",
        "GIT_SSH",
        "GIT_ASKPASS",
        "GIT_PROXY_COMMAND",
        "SSH_ASKPASS",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_EXEC_PATH",
        "GIT_NAMESPACE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(name);
    }
    // Named by the caller, so the sweep is testable without mutating the process environment.
    for name in env_names {
        let text = name.to_string_lossy();
        if text.starts_with("GIT_CONFIG_KEY_") || text.starts_with("GIT_CONFIG_VALUE_") {
            command.env_remove(&name);
        }
    }
    command
}

/// Per-stream cap, so a child printing forever cannot grow memory without bound.
const OUTPUT_LIMIT: usize = 1 << 24;

/// Read one stream to one past its cap, so the caller can tell a full stream
/// from an over-long one and act on the report.
fn read_capped(reader: Box<dyn Read + Send>, cap: usize) -> (Vec<u8>, bool) {
    let mut buffer = Vec::new();
    let _ = reader.take(cap as u64 + 1).read_to_end(&mut buffer);
    let over = buffer.len() > cap;
    (buffer, over)
}

/// Bounds what `output()` would buffer without limit.
fn run_capped(command: &mut Command, cap: usize) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn()?;
    capture(child, cap)
}

fn capture(mut child: Child, cap: usize) -> io::Result<Output> {
    let streams = take_piped(child.stdout.take(), "stdout").and_then(|stdout| {
        take_piped(child.stderr.take(), "stderr").map(|stderr| (stdout, stderr))
    });
    let (stdout, stderr) = match streams {
        Ok(streams) => streams,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let out_sender = sender.clone();
    std::thread::spawn(move || {
        let _ = out_sender.send((true, read_capped(Box::new(stdout), cap)));
    });
    std::thread::spawn(move || {
        let _ = sender.send((false, read_capped(Box::new(stderr), cap)));
    });
    let (stdout, stderr) = collect_readers(&receiver, &mut child);
    if stdout.1 || stderr.1 {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other(format!(
            "output limit of {cap} bytes was exceeded"
        )));
    }
    child.wait().map(|status| Output {
        status,
        stdout: stdout.0,
        stderr: stderr.0,
    })
}

/// How long the other reader gets once the cap trips: the killed child's pipe
/// closes at once, so only a descendant holding it open can use this up.
const READER_GRACE: Duration = Duration::from_secs(2);

/// A reader that vanishes without reporting leaves an empty result.
fn collect_readers(
    receiver: &std::sync::mpsc::Receiver<(bool, (Vec<u8>, bool))>,
    child: &mut Child,
) -> ((Vec<u8>, bool), (Vec<u8>, bool)) {
    let mut stdout = None;
    let mut stderr = None;
    let mut over = false;
    while stdout.is_none() || stderr.is_none() {
        let received = if over {
            receiver.recv_timeout(READER_GRACE).ok()
        } else {
            receiver.recv().ok()
        };
        let Some((is_stdout, result)) = received else {
            break;
        };
        if result.1 {
            over = true;
            let _ = child.kill();
        }
        if is_stdout {
            stdout = Some(result);
        } else {
            stderr = Some(result);
        }
    }
    (stdout.unwrap_or_default(), stderr.unwrap_or_default())
}

/// Take a stream that spawn just piped. A missing one breaks that setup, so it is
/// an error here rather than a panic at the call site.
fn take_piped<T>(stream: Option<T>, name: &str) -> io::Result<T> {
    stream.ok_or_else(|| io::Error::other(format!("child {name} was not piped")))
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Sys for RealSys {
    /// Pin the C locale, as the update logic parses git's English messages.
    fn git(&self, dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let env_names = std::env::vars_os().map(|(name, _)| name);
        run_capped(&mut git_command(dir, argv, env_names), OUTPUT_LIMIT)
    }

    /// Run the user's program without a shell, so paths and arguments are never reinterpreted.
    fn exec(&self, dir: &Path, argv: &[String]) -> io::Result<Output> {
        let mut command = base(dir, &argv[0]);
        command.args(&argv[1..]);
        run_capped(&mut command, OUTPUT_LIMIT)
    }

    /// Read straight from disk, behind the seam so tests can fail reads on cue.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    /// Write a temp sibling then rename, so a failed write never leaves a partial target.
    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }
        // Pid plus counter: concurrent runs never share a temp file.
        let temp_name = format!(
            "9up.tmp-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let temp = parent.map_or_else(|| PathBuf::from(&temp_name), |p| p.join(&temp_name));
        let permissions = fs::metadata(path).ok().map(|m| m.permissions());
        // `create_new`: a pre-planted file or link at the temp path fails the write
        // instead of being followed or replaced.
        let result = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .and_then(|mut file| {
                if let Some(permissions) = &permissions {
                    let _ = fs::set_permissions(&temp, permissions.clone());
                }
                file.write_all(contents)
            })
            .and_then(|()| fs::rename(&temp, path));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

#[cfg(test)]
mod tests;

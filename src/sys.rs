use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::other(format!(
            "no read in this stub: {}",
            path.display()
        )))
    }

    /// Write a file, creating its parent directory when it has one.
    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let _ = contents;
        Err(io::Error::other(format!(
            "no write in this stub: {}",
            path.display()
        )))
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

impl Sys for RealSys {
    /// Pin the C locale, as the update logic parses git's English messages.
    fn git(&self, dir: &Path, argv: &[&str]) -> io::Result<Output> {
        base(dir, "git")
            .args(argv)
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C")
            .output()
    }

    /// Run the user's program without a shell, so paths and arguments are never reinterpreted.
    fn exec(&self, dir: &Path, argv: &[String]) -> io::Result<Output> {
        base(dir, &argv[0]).args(&argv[1..]).output()
    }

    /// Read straight from disk, behind the seam so tests can fail reads on cue.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    /// Create the parent first, so the first config save works before its directory exists.
    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }
}

#[cfg(test)]
mod tests;

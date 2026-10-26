use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The application context. `main` builds it once — the system
/// services, the resolved values and the output — and passes it down.
/// Nothing below `main` reads the environment or builds a service
/// (in production code; tests build stubs).
pub struct App<'a> {
    pub sys: Box<dyn Sys>,
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
    pub bookmark_file: PathBuf,
    pub colour: bool,
    pub out: &'a mut (dyn io::Write + 'a),
}

/// The operating-system services the program uses. Tests substitute a
/// stub to force the failures a real system will not produce on cue.
pub trait Sys {
    /// Run git in `dir`: the locale is pinned to English (the branch
    /// logic reads git's words) and standard input is null. The
    /// default fails: a test stub overrides only what it scripts.
    fn git(&self, _dir: &Path, _argv: &[&str]) -> io::Result<Output> {
        Err(io::Error::other("not implemented in this stub"))
    }

    /// Run the user's command in `dir`, capturing its output. The exit
    /// status is the caller's to ignore.
    fn exec(&self, _dir: &Path, _argv: &[String]) -> io::Result<Output> {
        Err(io::Error::other("not implemented in this stub"))
    }

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

/// The real services, over the operating system.
pub struct RealSys;

fn base(dir: &Path, program: &str) -> Command {
    let mut command = Command::new(program);
    command.current_dir(dir).stdin(Stdio::null());
    command
}

impl Sys for RealSys {
    fn git(&self, dir: &Path, argv: &[&str]) -> io::Result<Output> {
        base(dir, "git")
            .args(argv)
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C")
            .output()
    }

    fn exec(&self, dir: &Path, argv: &[String]) -> io::Result<Output> {
        base(dir, &argv[0]).args(&argv[1..]).output()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }
}

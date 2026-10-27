mod cli;
mod commands;
mod config;
mod discover;
mod render;
mod sys;
mod update;

use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command, UpdateArgs};
use sys::{App, RealSys};

fn main() -> ExitCode {
    let cli = Cli::parse_from(cli::argv());
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if let Err(err) = writeln!(out, "9up").and_then(|()| writeln!(out)) {
        return if err.kind() == io::ErrorKind::BrokenPipe {
            ExitCode::SUCCESS
        } else {
            eprintln!("9up: {err}");
            ExitCode::FAILURE
        };
    }
    // The composition root: every service and resolved value is built
    // here and passed down in the context.
    // `list` and `clean` never need the cwd (configured paths are
    // absolute), so a deleted or unreadable cwd must not fail them.
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => match &cli.command {
            Some(Command::List) | Some(Command::Clean) => PathBuf::from("/"),
            _ => {
                eprintln!("9up: {err}");
                return ExitCode::FAILURE;
            }
        },
    };
    let config_dir = dirs::config_dir();
    let config_file = config::file_path(cli.config_file.as_deref(), config_dir.as_deref());
    let mut app = App {
        sys: Box::new(RealSys),
        cwd,
        home: dirs::home_dir(),
        config_file,
        colour: io::stdout().is_terminal(),
        out: &mut out,
    };
    match run(&mut app, &cli) {
        Ok(code) => code,
        // A closed pipe ends the run quietly, with the success code.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("9up: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(app: &mut App, cli: &Cli) -> io::Result<ExitCode> {
    match &cli.command {
        Some(Command::Add { paths }) => commands::add(app, paths),
        Some(Command::Delete { paths }) => commands::delete(app, paths),
        Some(Command::List) => commands::list(app),
        Some(Command::Clean) => commands::clean(app),
        Some(Command::Update(args)) => commands::update(app, args),
        None => commands::update(
            app,
            &UpdateArgs {
                depth: 3,
                ..Default::default()
            },
        ),
        Some(Command::Exec {
            paths,
            depth,
            command,
        }) => commands::exec(app, paths, *depth, command),
    }
}

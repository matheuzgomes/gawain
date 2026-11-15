use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

use gawain::check::{self, CheckRun};
use gawain::config::{Mode, TypeChecker};
use gawain::presence;
use gawain::{config, discovery, report};

const OUTPUT_BUFFER: usize = 64 * 1024;
const PROBLEMS_FOUND: u8 = 1;
const CANNOT_CHECK: u8 = 2;

/// Checks Python source files for missing type annotations.
#[derive(Debug, Parser)]
#[command(name = "gawain", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check Python files and directories
    Check(CheckArgs),
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Python files or directories to check
    #[arg(required = true, value_name = "PATHS")]
    paths: Vec<PathBuf>,

    /// Skip the types phase, which runs the type checker on the files once presence is clean
    #[arg(long)]
    no_types: bool,

    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// How obvious values are judged. Overrides `mode` in `[tool.gawain]`
    #[arg(long, value_enum)]
    mode: Option<Mode>,

    /// The type checker for the types phase. Overrides `checker` in `[tool.gawain]`
    #[arg(long, value_enum)]
    checker: Option<TypeChecker>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

/// Runs gawain on a thread whose stack is `presence::STACK_SIZE`, so that a deeply nested file cannot
/// overflow the stack of the main thread.
fn main() -> ExitCode {
    let worker = std::thread::Builder::new()
        .stack_size(presence::STACK_SIZE)
        .spawn(run);
    match worker {
        Ok(handle) => finish(handle),
        Err(error) => {
            eprintln!("gawain: cannot start: {error}");
            ExitCode::from(CANNOT_CHECK)
        }
    }
}

/// The status that the thread of `run` returns. A panic on that thread is raised again here, so that the
/// process ends as it would after a panic on the main thread.
fn finish(handle: std::thread::JoinHandle<ExitCode>) -> ExitCode {
    match handle.join() {
        Ok(status) => status,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check(args) => run_check(args),
    }
}

/// Exit status: 0 when nothing is reported or the type checker reports only warnings, 1 when presence finds
/// a problem (a file that cannot be read or parsed, or a directory that cannot be walked, counts) or the
/// type checker reports an error, 2 on usage or when the type checker cannot run.
fn run_check(args: CheckArgs) -> ExitCode {
    match check(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(PROBLEMS_FOUND),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(CANNOT_CHECK)
        }
    }
}

/// Whether the check passed. A usage error, a type checker that cannot run or output that cannot be written
/// is returned as the message to print.
fn check(args: CheckArgs) -> Result<bool, String> {
    let CheckArgs {
        paths,
        no_types,
        format,
        mode,
        checker,
    } = args;

    ensure_paths_exist(&paths)?;
    let config = config::load().map_err(|error| format!("gawain: {error}"))?;
    let mode = mode.unwrap_or(config.mode);
    let checker = (!no_types).then(|| checker.unwrap_or(config.checker));
    let found = discover(&paths, &config.exclude);
    let run = check::run(found, mode, checker).map_err(|error| error.to_string())?;
    write_report(&run, format)?;
    Ok(run.passed())
}

/// When there are none, the notice goes to stderr, and the run passes unless a walk error reports a path
/// it could not read.
fn discover(paths: &[PathBuf], exclude: &[String]) -> discovery::Discovery {
    let found = discovery::python_files(paths, exclude);
    if found.files.is_empty() {
        eprintln!("gawain: no Python files found in the given paths");
    }
    found
}

fn ensure_paths_exist(paths: &[PathBuf]) -> Result<(), String> {
    match paths.iter().find(|path| !path.exists()) {
        Some(missing) => Err(format!(
            "gawain: no such file or directory: {}",
            missing.display()
        )),
        None => Ok(()),
    }
}

fn write_report(run: &CheckRun, format: Format) -> Result<(), String> {
    let mut out = BufWriter::with_capacity(OUTPUT_BUFFER, io::stdout().lock());
    match format {
        Format::Text => report::write_text(&mut out, run).map_err(output_error)?,
        Format::Json => write_json(&mut out, run)?,
    }
    writeln!(out)
        .and_then(|()| out.flush())
        .map_err(output_error)
}

/// Only the rendering can fail, and that happens before anything is written.
fn write_json(out: &mut impl Write, run: &CheckRun) -> Result<(), String> {
    let json =
        report::render_json(run).map_err(|error| format!("gawain: cannot render JSON: {error}"))?;
    out.write_all(json.as_bytes()).map_err(output_error)
}

fn output_error(error: io::Error) -> String {
    format!("gawain: cannot write output: {error}")
}

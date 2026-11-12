use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

use gawain::config::{Mode, TypeChecker};
use gawain::diagnostics::Diagnostic;
use gawain::presence;
use gawain::types_phase::{self, TypesError, TypesRun};
use gawain::{config, discovery, report};

const OUTPUT_BUFFER: usize = 64 * 1024;

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
            ExitCode::from(2)
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
        Ok(status) => status,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

/// A usage error or a type checker that cannot run is returned as the message to print.
fn check(args: CheckArgs) -> Result<ExitCode, String> {
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
    let checker = checker.unwrap_or(config.checker);
    let found = discover(&paths, &config.exclude);
    let (presence, types) = run_phases(found, no_types, mode, checker)?;
    Ok(print_report(&presence, types.as_ref(), format))
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

fn run_phases(
    found: discovery::Discovery,
    no_types: bool,
    mode: Mode,
    checker: TypeChecker,
) -> Result<(Vec<Diagnostic>, Option<TypesRun>), String> {
    let presence = presence::check_files(&found.files, found.errors, mode)
        .map_err(|error| format!("gawain: {error}"))?;
    let types = run_types_if_due(&found.files, &presence, no_types, checker)
        .map_err(|error| error.to_string())?;
    Ok((presence, types))
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

/// Runs the types phase with `checker` on `files` when it is due: presence is clean, `--no-types` is not
/// given, and there is at least one file. Returns `None` when it is not due.
fn run_types_if_due(
    files: &[PathBuf],
    presence: &[Diagnostic],
    no_types: bool,
    checker: TypeChecker,
) -> Result<Option<TypesRun>, TypesError> {
    if !presence.is_empty() || no_types || files.is_empty() {
        return Ok(None);
    }
    types_phase::run(checker, files).map(Some)
}

fn print_report(presence: &[Diagnostic], types: Option<&TypesRun>, format: Format) -> ExitCode {
    match write_report(presence, types, format) {
        Ok(()) => exit_status(presence, types),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

/// Only the JSON form can fail to render.
fn write_report(
    presence: &[Diagnostic],
    types: Option<&TypesRun>,
    format: Format,
) -> Result<(), String> {
    let mut out = BufWriter::with_capacity(OUTPUT_BUFFER, io::stdout().lock());
    write_format(&mut out, presence, types, format)?;
    writeln!(out)
        .and_then(|()| out.flush())
        .map_err(output_error)
}

fn write_format(
    out: &mut impl Write,
    presence: &[Diagnostic],
    types: Option<&TypesRun>,
    format: Format,
) -> Result<(), String> {
    match format {
        Format::Text => report::write_text(out, presence, types).map_err(output_error),
        Format::Json => write_json(out, presence, types),
    }
}

/// Writes the JSON report. Only the rendering can fail, and that happens before anything is written.
fn write_json(
    out: &mut impl Write,
    presence: &[Diagnostic],
    types: Option<&TypesRun>,
) -> Result<(), String> {
    let json = report::render_json(presence, types)
        .map_err(|error| format!("gawain: cannot render JSON: {error}"))?;
    out.write_all(json.as_bytes()).map_err(output_error)
}

fn output_error(error: io::Error) -> String {
    format!("gawain: cannot write output: {error}")
}

fn exit_status(presence: &[Diagnostic], types: Option<&TypesRun>) -> ExitCode {
    let types_failed = types.is_some_and(|run| run.failed);
    if presence.is_empty() && !types_failed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

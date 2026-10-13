//! `bodo-cli`: Bodo from the command line. Run `bodo-cli --help` for the commands.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use bodo_cli::link::{self, Target};
use bodo_cli::render::{RenderOptions, render, render_stems};
use bodo_cli::session;
use bodo_engine::{Registry, inspect};

const USAGE: &str = "\
usage: bodo-cli <command> [arguments]

commands:
  mcp [project.bodo]          serve Bodo's MCP tools over stdio on a project (a new one by default)
  mcp --connect [port]        serve the tools over stdio, acting on the running Bodo app
  render <project> <out.wav>  render the song to a 32-bit float WAV file
      --stems <folder>        also render one WAV file per insert there (the out.wav may then be left out)
      --pattern <pattern>     loop one pattern instead (by name, number or id)
      --end <bar:beat:tick>   stop there instead of at the end of the song or pattern
  inspect <project>           print the project as text
  commands [namespace]        list the commands agents and the app run
  pack <dir.bodoproj> <file.bodo>    pack a project folder into a .bodo file
  unpack <file.bodo> <dir.bodoproj>  unpack a .bodo file into a project folder

options:
  -h, --help                  print this help
  -V, --version               print the version

Logs go to stderr; set BODO_LOG=debug for more. Stdout belongs to the MCP connection.";

fn main() -> ExitCode {
    init_logging();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Logs to stderr at the level `BODO_LOG` names, warnings by default.
fn init_logging() {
    let level = match std::env::var("BODO_LOG").unwrap_or_default().to_ascii_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::WARN,
    };
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_max_level(level).init();
}

/// Writes `text` to stdout.
fn out(text: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        stdout.write_all(b"\n")?;
    }
    Ok(())
}

fn run(args: &[String]) -> Result<()> {
    let (command, rest) = match args.split_first() {
        Some((command, rest)) => (command.as_str(), rest),
        None => return out(USAGE),
    };
    match command {
        "-h" | "--help" | "help" => out(USAGE),
        "-V" | "--version" => out(concat!("bodo-cli ", env!("CARGO_PKG_VERSION"))),
        "mcp" => mcp(rest),
        "render" => render_command(rest),
        "inspect" => {
            let [path] = rest else { bail!("usage: bodo-cli inspect <project>") };
            let session = session::open(Some(&PathBuf::from(path)))?;
            out(&inspect::project(&session).text)
        }
        "commands" => {
            let registry = Registry::builtin();
            let namespace = rest.first().map(String::as_str);
            let lines: Vec<String> = registry
                .iter()
                .filter(|c| namespace.is_none_or(|ns| c.namespace() == ns))
                .map(|c| format!("{}: {} [{}]", c.id, c.summary, c.effects.describe()))
                .collect();
            if lines.is_empty() {
                bail!("no commands in namespace `{}`", namespace.unwrap_or_default());
            }
            out(&lines.join("\n"))
        }
        "pack" | "unpack" => {
            let [from, to] = rest else { bail!("usage: bodo-cli {command} <from> <to>") };
            bodo_io::convert(&PathBuf::from(from), &PathBuf::from(to)).with_context(|| format!("cannot {command} {from}"))?;
            out(&format!("{command}ed {from} into {to}"))
        }
        other => bail!("unknown command `{other}`\n\n{USAGE}"),
    }
}

fn mcp(args: &[String]) -> Result<()> {
    match args {
        [] => Ok(bodo_mcp::serve_stdio(bodo_mcp::Server::new(session::open(None)?)?)?),
        [flag] if flag == "--connect" => Ok(bodo_mcp::serve_stdio(link::connected_server(Target::Running)?)?),
        [flag, port] if flag == "--connect" => {
            let port: u16 = port.parse().with_context(|| format!("`{port}` is not a port"))?;
            Ok(bodo_mcp::serve_stdio(link::connected_server(Target::Port(port))?)?)
        }
        [path] if !path.starts_with('-') => Ok(bodo_mcp::serve_stdio(bodo_mcp::Server::new(session::open(Some(&PathBuf::from(path)))?)?)?),
        _ => bail!("usage: bodo-cli mcp [project.bodo] | bodo-cli mcp --connect [port]"),
    }
}

fn render_command(args: &[String]) -> Result<()> {
    const RENDER_USAGE: &str = "usage: bodo-cli render <project> [<out.wav>] [--stems <folder>] [--pattern <pattern>] [--end <position>]";
    let mut paths = Vec::new();
    let mut stems = None;
    let mut options = RenderOptions::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--pattern" => options.pattern = Some(rest.next().context("--pattern needs a pattern")?.clone()),
            "--end" => options.end = Some(rest.next().context("--end needs a position such as 9:1:0")?.clone()),
            "--stems" => stems = Some(PathBuf::from(rest.next().context("--stems needs a folder")?)),
            flag if flag.starts_with("--") => bail!("unknown option `{flag}`"),
            path => paths.push(PathBuf::from(path)),
        }
    }
    let (project, wav) = match paths.as_slice() {
        [project] if stems.is_some() => (project, None),
        [project, wav] => (project, Some(wav)),
        _ => bail!(RENDER_USAGE),
    };
    if let Some(wav) = wav {
        let rendered = render(project, wav, &options)?;
        let _ = writeln!(std::io::stderr(), "rendered {:.2} s to {}, peak {:.1} dBFS", rendered.seconds(), wav.display(), rendered.peak_db);
    }
    if let Some(folder) = stems {
        let files = render_stems(project, &folder, &options)?;
        let _ = writeln!(std::io::stderr(), "rendered {} stems to {}", files.len(), folder.display());
    }
    Ok(())
}

use std::process::ExitCode;

use log::{LevelFilter, Log, Metadata, Record};

const USAGE: &str = "usage: kotlin-navigator [--version | --bench <root>]";

/// Logs go to stderr: stdout carries only LSP messages.
struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            eprintln!("kotlin-navigator {}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

fn init_logger() {
    let level = std::env::var("KOTLIN_NAVIGATOR_LOG")
        .ok()
        .and_then(|v| v.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Warn);
    if log::set_logger(&StderrLogger).is_ok() {
        log::set_max_level(level);
    }
}

fn serve() -> ExitCode {
    init_logger();
    let (connection, io_threads) = lsp_server::Connection::stdio();
    let served = kotlin_navigator::lsp::run(connection);
    let joined = io_threads.join();
    match (served, joined) {
        (Ok(()), Ok(())) => ExitCode::SUCCESS,
        (Err(e), _) => {
            log::error!("{e:#}");
            ExitCode::FAILURE
        }
        (_, Err(e)) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => serve(),
        [flag, root] if flag == "--bench" => match kotlin_navigator::bench::run(std::path::Path::new(root)) {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e:#}");
                ExitCode::FAILURE
            }
        },
        [flag] if flag == "--version" => {
            println!("kotlin-navigator {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

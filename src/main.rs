use std::process::ExitCode;

const USAGE: &str = "usage: kotlin-navigator [--version | --bench <root>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => ExitCode::SUCCESS,
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

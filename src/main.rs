mod error;
mod format;
mod meta;
mod png;

use std::io::Read as _;

use error::Error;

fn main() {
    if let Err(err) = run() {
        eprintln!("ime: {err}");
        std::process::exit(err.exit_code());
    }
}

fn run() -> Result<(), Error> {
    let mut file: Option<String> = None;
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next().map_err(|err| Error::usage(err.to_string()))? {
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                print_help();
                return Ok(());
            }
            lexopt::Arg::Short('v') | lexopt::Arg::Long("version") => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            lexopt::Arg::Value(path) => {
                if file.is_some() {
                    return Err(Error::usage("expected a single <file> argument"));
                }
                file = Some(path.to_string_lossy().into_owned());
            }
            lexopt::Arg::Short(flag) => return Err(Error::usage(format!("unexpected argument '-{flag}'"))),
            lexopt::Arg::Long(flag) => return Err(Error::usage(format!("unexpected argument '--{flag}'"))),
        }
    }

    let Some(file) = file else {
        return Err(Error::runtime("no file given: interactive TUI mode is not implemented yet"));
    };
    let display = if file == "-" { "<stdin>".to_string() } else { format!("'{file}'") };

    let bytes = read_input(&file)?;
    let detected = format::detect(&bytes).ok_or_else(|| Error::runtime(format!("unsupported image format in {display}: expected PNG, JPEG, or WebP (detected by magic bytes)")))?;
    let metadata = meta::read_metadata(&bytes, detected)?;
    let json = serde_json::to_string_pretty(&metadata.to_value()).map_err(|err| Error::runtime(format!("cannot encode metadata as JSON: {err}")))?;
    println!("{json}");
    Ok(())
}

fn read_input(file: &str) -> Result<Vec<u8>, Error> {
    if file == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).map_err(|err| Error::runtime(format!("cannot read <stdin>: {err}")))?;
        return Ok(bytes);
    }
    std::fs::read(file).map_err(|err| Error::runtime(format!("cannot read '{file}': {err}")))
}

fn print_help() {
    println!("ime {} — read, write, and wipe metadata in PNG, JPEG, and WebP files.", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Usage:");
    println!("  ime <file>         Print all metadata as JSON to stdout (use - for stdin)");
    println!("  ime --help         Print this help and exit");
    println!("  ime --version      Print the version number and exit");
}

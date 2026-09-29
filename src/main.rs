mod error;
mod exif_tags;
mod exif_write;
mod format;
mod jpeg;
mod merge;
mod meta;
mod png;
mod webp;
mod write;

use std::io::Read as _;

use error::Error;

enum Operation {
    Read,
    Set(Vec<String>),
    Wipe,
}

enum Output {
    InPlace,
    Path(String),
    Stdout,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("ime: {err}");
        std::process::exit(err.exit_code());
    }
}

fn run() -> Result<(), Error> {
    let mut file: Option<String> = None;
    let mut sets: Vec<String> = Vec::new();
    let mut wipe = false;
    let mut output: Option<String> = None;
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next().map_err(|err| Error::usage(err.to_string()))? {
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                print_help()?;
                return Ok(());
            }
            lexopt::Arg::Short('v') | lexopt::Arg::Long("version") => {
                write::write_stdout(format!("{}\n", env!("CARGO_PKG_VERSION")).as_bytes())?;
                return Ok(());
            }
            lexopt::Arg::Short('s') | lexopt::Arg::Long("set") => {
                let value = parser.value().map_err(|_| Error::usage("--set requires a JSON argument"))?;
                sets.push(value.to_string_lossy().into_owned());
            }
            lexopt::Arg::Short('w') | lexopt::Arg::Long("wipe") => wipe = true,
            lexopt::Arg::Short('o') | lexopt::Arg::Long("output") => {
                let value = parser.value().map_err(|_| Error::usage("--output requires a path argument"))?;
                output = Some(value.to_string_lossy().into_owned());
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

    if !sets.is_empty() && wipe {
        return Err(Error::usage("--set and --wipe are mutually exclusive"));
    }
    let writing = !sets.is_empty() || wipe;
    if output.is_some() && !writing {
        return Err(Error::usage("--output requires --set or --wipe"));
    }
    let Some(file) = file else {
        if writing || output.is_some() {
            return Err(Error::usage("missing <file> argument"));
        }
        return Err(Error::runtime("no file given: interactive TUI mode is not implemented yet"));
    };

    let operation = if wipe {
        Operation::Wipe
    } else if sets.is_empty() {
        Operation::Read
    } else {
        Operation::Set(sets)
    };
    let output = match output.as_deref() {
        None => Output::InPlace,
        Some("-") => Output::Stdout,
        Some(path) => Output::Path(path.to_string()),
    };

    let display = if file == "-" { "<stdin>".to_string() } else { format!("'{file}'") };
    let bytes = read_input(&file)?;
    let detected = format::detect(&bytes).ok_or_else(|| Error::runtime(format!("unsupported image format in {display}: expected PNG, JPEG, or WebP (detected by magic bytes)")))?;

    match operation {
        Operation::Read => {
            let metadata = meta::read_metadata(&bytes, detected)?;
            let mut json = serde_json::to_string_pretty(&metadata.to_value()).map_err(|err| Error::runtime(format!("cannot encode metadata as JSON: {err}")))?;
            json.push('\n');
            write::write_stdout(json.as_bytes())
        }
        Operation::Set(payloads) => {
            let outcome = write::apply_set(&bytes, detected, &payloads)?;
            finish_write(&file, output, &bytes, outcome, detected)
        }
        Operation::Wipe => {
            let outcome = write::apply_wipe(&bytes, detected)?;
            finish_write(&file, output, &bytes, outcome, detected)
        }
    }
}

/// Verify the write result against the source, then route it to its
/// destination. An unchanged in-place result skips the write entirely.
fn finish_write(file: &str, output: Output, source: &[u8], outcome: write::WriteOutcome, format: format::ImageFormat) -> Result<(), Error> {
    let in_place_noop = !outcome.changed && matches!(output, Output::InPlace);
    if in_place_noop {
        return Ok(());
    }
    write::verify(source, &outcome.bytes, format)?;
    match output {
        Output::InPlace => write::write_atomic(std::path::Path::new(file), &outcome.bytes),
        Output::Path(path) => write::write_atomic(std::path::Path::new(&path), &outcome.bytes),
        Output::Stdout => write::write_stdout(&outcome.bytes),
    }
}

fn read_input(file: &str) -> Result<Vec<u8>, Error> {
    if file == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).map_err(|err| Error::runtime(format!("cannot read <stdin>: {err}")))?;
        return Ok(bytes);
    }
    std::fs::read(file).map_err(|err| Error::runtime(format!("cannot read '{file}': {err}")))
}

fn print_help() -> Result<(), Error> {
    let text = format!(
        "ime {} — read, write, and wipe metadata in PNG, JPEG, and WebP files.\n\nUsage:\n  ime <file>                   Print all metadata as JSON to stdout (use - for stdin)\n  ime <file> --set <JSON>      Merge a JSON object into the file's metadata (repeatable)\n  ime <file> --wipe            Remove all metadata from the file\n  ime <file> --set <JSON> -o <path>  Write the result to <path> instead (use - for stdout)\n  ime --help                   Print this help and exit\n  ime --version                Print the version number and exit\n",
        env!("CARGO_PKG_VERSION")
    );
    write::write_stdout(text.as_bytes())
}

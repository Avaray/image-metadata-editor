mod batch;
mod error;
mod exif_tags;
mod exif_write;
mod format;
mod jpeg;
mod merge;
mod meta;
mod png;
mod tui;
mod tui_tree;
mod tui_ui;
mod webp;
mod write;

use std::io::Read as _;

use error::Error;

enum Operation {
    Read,
    Set(Vec<merge::SetPayload>),
    Wipe,
}

enum Output {
    InPlace,
    Path(String),
    Stdout,
}

/// A batch write/--dry-run operation, borrowed from the parsed CLI operation.
enum BatchOp<'a> {
    Set(&'a [merge::SetPayload]),
    Wipe,
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
    let mut dry_run = false;
    let mut recursive = false;
    let mut tui = false;
    let mut power = false;
    let mut watch = false;
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
            lexopt::Arg::Long("dry-run") => dry_run = true,
            lexopt::Arg::Short('r') | lexopt::Arg::Long("recursive") => recursive = true,
            lexopt::Arg::Short('t') | lexopt::Arg::Long("tui") => tui = true,
            lexopt::Arg::Short('p') | lexopt::Arg::Long("power") => power = true,
            lexopt::Arg::Long("watch") => watch = true,
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
    if dry_run && !writing {
        return Err(Error::usage("--dry-run requires --set or --wipe"));
    }
    if dry_run && output.is_some() {
        return Err(Error::usage("--dry-run and --output are mutually exclusive"));
    }
    if output.is_some() && recursive {
        return Err(Error::usage("--output cannot be used with --recursive"));
    }
    if sets.iter().filter(|set| set.as_str() == "-").count() > 1 {
        return Err(Error::usage("at most one --set - may appear per invocation"));
    }
    if tui && (writing || output.is_some() || dry_run || recursive) {
        return Err(Error::usage("--tui cannot be combined with --set, --wipe, --dry-run, --output, or --recursive"));
    }
    let Some(file) = file else {
        if writing || output.is_some() || dry_run || recursive {
            return Err(Error::usage("missing <file> argument"));
        }
        // Bare launch (optionally with --tui/--power/--watch): TUI in the
        // current directory.
        let cwd = std::env::current_dir().map_err(|err| Error::runtime(format!("cannot open current directory: {err}")))?;
        return tui::run(&cwd, power, watch);
    };
    if tui {
        if file == "-" {
            return Err(Error::usage("--tui cannot be used with <file> - (stdin)"));
        }
        return tui::run(std::path::Path::new(&file), power, watch);
    }
    if watch {
        return Err(Error::usage("--watch requires TUI mode (--tui or no arguments)"));
    }
    if power {
        return Err(Error::usage("--power requires TUI mode (--tui or no arguments)"));
    }
    if file == "-" && recursive {
        return Err(Error::usage("--recursive cannot be used with <file> - (stdin)"));
    }
    if file == "-" && sets.iter().any(|set| set == "-") {
        return Err(Error::usage("stdin cannot serve both <file> and --set at once"));
    }

    // Resolve `--set` sources (inline, `-`/stdin, `@<path>`/file), then
    // validate every payload once, before any file is read or touched.
    let mut payloads = Vec::with_capacity(sets.len());
    for set in &sets {
        payloads.push(merge::parse_set_payload(&resolve_payload(set)?)?);
    }

    let operation = if wipe {
        Operation::Wipe
    } else if payloads.is_empty() {
        Operation::Read
    } else {
        Operation::Set(payloads)
    };
    let output = match output.as_deref() {
        // A stdin write has no in-place target: like the piped example in
        // `02-cli-interface.md`, the result goes to stdout by default.
        None if file == "-" && writing && !dry_run => Output::Stdout,
        None => Output::InPlace,
        Some("-") => Output::Stdout,
        Some(path) => Output::Path(path.to_string()),
    };

    if file == "-" {
        let bytes = read_input(&file)?;
        return run_single("<stdin>", None, &bytes, operation, output, dry_run);
    }
    let root = std::path::Path::new(&file);
    if matches!(std::fs::metadata(root), Ok(kind) if kind.is_dir()) {
        if !matches!(output, Output::InPlace) {
            return Err(Error::usage("--output cannot be used with a directory"));
        }
        return run_batch(root, recursive, operation, dry_run);
    }
    let bytes = read_input(&file)?;
    run_single(&format!("'{file}'"), Some(root), &bytes, operation, output, dry_run)
}

fn run_single(display: &str, in_place: Option<&std::path::Path>, bytes: &[u8], operation: Operation, output: Output, dry_run: bool) -> Result<(), Error> {
    let detected = format::detect(bytes).ok_or_else(|| Error::runtime(format!("unsupported image format in {display}: expected PNG, JPEG, or WebP (detected by magic bytes)")))?;
    match operation {
        Operation::Read => {
            let metadata = meta::read_metadata(bytes, detected)?;
            print_json(&metadata.to_value())
        }
        Operation::Set(payloads) => {
            let outcome = write::apply_set(bytes, detected, &payloads)?;
            if dry_run { finish_dry_run(bytes, &outcome, detected) } else { finish_write(in_place, output, bytes, outcome, detected) }
        }
        Operation::Wipe => {
            let outcome = write::apply_wipe(bytes, detected)?;
            if dry_run { finish_dry_run(bytes, &outcome, detected) } else { finish_write(in_place, output, bytes, outcome, detected) }
        }
    }
}

/// Verify the write result against the source, then route it to its
/// destination. An unchanged in-place result skips the write entirely.
fn finish_write(in_place: Option<&std::path::Path>, output: Output, source: &[u8], outcome: write::WriteOutcome, format: format::ImageFormat) -> Result<(), Error> {
    let in_place_noop = !outcome.changed && matches!(output, Output::InPlace);
    if in_place_noop {
        return Ok(());
    }
    write::verify(source, &outcome.bytes, format)?;
    match output {
        Output::InPlace => match in_place {
            // Stdin writes always resolve to `Output::Stdout`, so an in-place
            // write with no path is unreachable by construction.
            None => unreachable!("stdin writes always resolve to --output -"),
            Some(path) => write::write_atomic(path, &outcome.bytes),
        },
        Output::Path(path) => write::write_atomic(std::path::Path::new(&path), &outcome.bytes),
        Output::Stdout => write::write_stdout(&outcome.bytes),
    }
}

/// A `--dry-run` write: verify the in-memory result exactly like a real write
/// would, then print the resulting metadata instead of writing anything.
fn finish_dry_run(source: &[u8], outcome: &write::WriteOutcome, format: format::ImageFormat) -> Result<(), Error> {
    write::verify(source, &outcome.bytes, format)?;
    let metadata = meta::read_metadata(&outcome.bytes, format)?;
    print_json(&metadata.to_value())
}

fn run_batch(dir: &std::path::Path, recursive: bool, operation: Operation, dry_run: bool) -> Result<(), Error> {
    let entries = batch::collect(dir, recursive);
    match operation {
        Operation::Read => batch_read(&entries),
        Operation::Set(payloads) => {
            let op = BatchOp::Set(&payloads);
            if dry_run { batch_dry_run(&entries, &op) } else { batch_write(&entries, &op) }
        }
        Operation::Wipe => {
            let op = BatchOp::Wipe;
            if dry_run { batch_dry_run(&entries, &op) } else { batch_write(&entries, &op) }
        }
    }
}

/// A batch read prints the aggregated `{ "<path>": <metadata> }` object to
/// stdout. Per-file failures go to stderr without stopping the batch; stdout
/// stays pure JSON. Any failure means exit `1` once the batch completes.
fn batch_read(entries: &[batch::Entry]) -> Result<(), Error> {
    let mut aggregated = serde_json::Map::new();
    let mut errors = 0u32;
    for entry in entries {
        match entry {
            batch::Entry::Failed { path, message } => {
                eprintln!("ERROR {path}: {message}");
                errors += 1;
            }
            batch::Entry::Work { path } => {
                let display = path.display().to_string();
                match read_batch_file(path) {
                    Ok(value) => {
                        aggregated.insert(display, value);
                    }
                    Err(err) => {
                        eprintln!("ERROR {display}: {err}");
                        errors += 1;
                    }
                }
            }
        }
    }
    print_json(&serde_json::Value::Object(aggregated))?;
    if errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// A batch `--dry-run` prints the same aggregated shape a batch read would,
/// holding every file's would-be result; nothing is written to disk.
fn batch_dry_run(entries: &[batch::Entry], op: &BatchOp) -> Result<(), Error> {
    let mut aggregated = serde_json::Map::new();
    let mut errors = 0u32;
    for entry in entries {
        match entry {
            batch::Entry::Failed { path, message } => {
                eprintln!("ERROR {path}: {message}");
                errors += 1;
            }
            batch::Entry::Work { path } => {
                let display = path.display().to_string();
                match dry_run_batch_file(path, op) {
                    Ok(value) => {
                        aggregated.insert(display, value);
                    }
                    Err(err) => {
                        eprintln!("ERROR {display}: {err}");
                        errors += 1;
                    }
                }
            }
        }
    }
    print_json(&serde_json::Value::Object(aggregated))?;
    if errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// A batch `--set`/`--wipe` modifies each supported file in place, with
/// per-file progress on stderr and a final summary line. One file's failure
/// never stops the batch; any failure means exit `1` after the summary.
fn batch_write(entries: &[batch::Entry], op: &BatchOp) -> Result<(), Error> {
    let total = entries.len();
    let mut succeeded = 0usize;
    let mut errors = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        let n = index + 1;
        match entry {
            batch::Entry::Failed { path, message } => {
                eprintln!("[{n}/{total}] ERROR {path}: {message}");
                errors += 1;
            }
            batch::Entry::Work { path } => {
                let display = path.display().to_string();
                match apply_batch_write(path, op) {
                    Ok(()) => {
                        eprintln!("[{n}/{total}] OK {display}");
                        succeeded += 1;
                    }
                    Err(err) => {
                        eprintln!("[{n}/{total}] ERROR {display}: {err}");
                        errors += 1;
                    }
                }
            }
        }
    }
    eprintln!("Done: {succeeded}/{total} succeeded, {errors} errors");
    if errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn read_batch_file(path: &std::path::Path) -> Result<serde_json::Value, Error> {
    let bytes = std::fs::read(path).map_err(|err| Error::runtime(format!("cannot read file: {err}")))?;
    let detected = format::detect(&bytes).ok_or_else(|| Error::runtime("unsupported image format: expected PNG, JPEG, or WebP (detected by magic bytes)"))?;
    Ok(meta::read_metadata(&bytes, detected)?.to_value())
}

fn dry_run_batch_file(path: &std::path::Path, op: &BatchOp) -> Result<serde_json::Value, Error> {
    let bytes = std::fs::read(path).map_err(|err| Error::runtime(format!("cannot read file: {err}")))?;
    let detected = format::detect(&bytes).ok_or_else(|| Error::runtime("unsupported image format: expected PNG, JPEG, or WebP (detected by magic bytes)"))?;
    let outcome = match op {
        BatchOp::Set(payloads) => write::apply_set(&bytes, detected, payloads)?,
        BatchOp::Wipe => write::apply_wipe(&bytes, detected)?,
    };
    write::verify(&bytes, &outcome.bytes, detected)?;
    Ok(meta::read_metadata(&outcome.bytes, detected)?.to_value())
}

fn apply_batch_write(path: &std::path::Path, op: &BatchOp) -> Result<(), Error> {
    let bytes = std::fs::read(path).map_err(|err| Error::runtime(format!("cannot read file: {err}")))?;
    let detected = format::detect(&bytes).ok_or_else(|| Error::runtime("unsupported image format: expected PNG, JPEG, or WebP (detected by magic bytes)"))?;
    let outcome = match op {
        BatchOp::Set(payloads) => write::apply_set(&bytes, detected, payloads)?,
        BatchOp::Wipe => write::apply_wipe(&bytes, detected)?,
    };
    if !outcome.changed {
        return Ok(());
    }
    write::verify(&bytes, &outcome.bytes, detected)?;
    write::write_atomic(path, &outcome.bytes)
}

/// Resolve one `--set` argument to its payload text: `-` reads stdin, `@<path>`
/// reads a file, anything else is an inline JSON5 string.
fn resolve_payload(set: &str) -> Result<String, Error> {
    if set == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).map_err(|err| Error::runtime(format!("cannot read --set payload from stdin: {err}")))?;
        return Ok(text);
    }
    if let Some(path) = set.strip_prefix('@') {
        return std::fs::read_to_string(path).map_err(|err| Error::runtime(format!("cannot read --set payload file '{path}': {err}")));
    }
    Ok(set.to_string())
}

fn print_json(value: &serde_json::Value) -> Result<(), Error> {
    let mut json = serde_json::to_string_pretty(value).map_err(|err| Error::runtime(format!("cannot encode metadata as JSON: {err}")))?;
    json.push('\n');
    write::write_stdout(json.as_bytes())
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
        "ime {} — read, write, and wipe metadata in PNG, JPEG, and WebP files.\n\nUsage:\n  ime <file>                          Print all metadata as JSON to stdout (use - for stdin)\n  ime <dir> [--recursive]             Print {{\"<path>\": <metadata>}} for every image in the directory\n  ime <file|dir> --set <JSON>         Merge a JSON object into the metadata (repeatable)\n  ime <file|dir> --wipe               Remove all metadata\n  ime <file|dir> --set <JSON> --dry-run  Print the would-be result without writing anything\n  ime <file> --set <JSON> -o <path>   Write the result to <path> instead (use - for stdout)\n  ime [--tui] [<path>]                Open the interactive TUI (default directory: current)\n  ime --help                          Print this help and exit\n  ime --version                       Print the version number and exit\n\n<JSON> is inline JSON5, - (read the payload from stdin), or @<path> (read it from a file).\n--set and --wipe are mutually exclusive; --output works on a single file only.\nWith stdin input and no --output, write results go to stdout.\n--power skips TUI confirmations; --watch live-refreshes the TUI file list.\n",
        env!("CARGO_PKG_VERSION")
    );
    write::write_stdout(text.as_bytes())
}

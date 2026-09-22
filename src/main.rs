mod cli;
mod error;
mod extract;
mod inject;
mod json_path;
mod model;
mod output;
mod strip;

use cli::CliResult;
use error::AppError;
use std::process;

fn run() -> Result<(), AppError> {
    match cli::parse_args().map_err(|e| AppError::Usage(e.to_string()))? {
        CliResult::Help => {
            println!(
                "Usage: ime <file> [-o|--output <path>] [-s|--strip] [--set Key=Value] [-k|--key <path>]"
            );
            println!("Options:");
            println!("  -o, --output <path>  write the result to <path> instead of stdout");
            println!(
                "  -s, --strip          strip metadata from the file (currently JPEG/PNG only)"
            );
            println!(
                "      --set K=V        set metadata Key to Value (can be used multiple times)"
            );
            println!("  -k, --key <path>     extract a single value using a jq-style path");
            println!("                       e.g. -k Exif.Model  or  -k PngText.prompt.steps[0]");
            println!(
                "                       if a string value looks like JSON, it is traversed too"
            );
            println!("  -v, --version        print version information");
            println!("  -h, --help           print this help message");
            Ok(())
        }
        CliResult::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        CliResult::Args(args) => {
            if args.strip {
                strip::strip_metadata(&args.file, args.output.as_deref())
                    .map_err(|e| AppError::Runtime(e.to_string()))?;
                return Ok(());
            }
            if !args.set.is_empty() {
                inject::inject_metadata(&args.file, args.output.as_deref(), &args.set)
                    .map_err(|e| AppError::Runtime(e.to_string()))?;
                return Ok(());
            }

            let mut parser = nom_exif::MediaParser::new();
            let metadata = extract::extract(&args.file, &mut parser)
                .map_err(|e| AppError::Runtime(e.to_string()))?;

            if let Some(key_path) = &args.key {
                // Serialize metadata to a Value first, then navigate the path
                let root: serde_json::Value = serde_json::to_value(&metadata)
                    .map_err(|e| AppError::Runtime(e.to_string()))?;
                match json_path::extract(root, key_path) {
                    Some(serde_json::Value::String(s)) => {
                        // Raw string — print without quotes, no trailing newline needed in non-tty
                        println!("{}", s);
                    }
                    Some(other) => {
                        // Object, array, number, bool — print as pretty JSON
                        let pretty = serde_json::to_string_pretty(&other)
                            .map_err(|e| AppError::Runtime(e.to_string()))?;
                        println!("{}", pretty);
                    }
                    None => {
                        return Err(AppError::Runtime(format!(
                            "Key '{}' not found in metadata",
                            key_path
                        )));
                    }
                }
                return Ok(());
            }

            output::write_output(&metadata, args.output.as_deref())
                .map_err(|e| AppError::Runtime(e.to_string()))?;
            Ok(())
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e);
        process::exit(e.exit_code());
    }
}

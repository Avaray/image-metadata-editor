mod cli;
mod error;
mod extract;
mod model;
mod output;
mod strip;

use cli::CliResult;
use error::AppError;
use std::process;

fn run() -> Result<(), AppError> {
    match cli::parse_args().map_err(|e| AppError::Usage(e.to_string()))? {
        CliResult::Help => {
            println!("Usage: mex <file> [-j|--json] [-o|--output <path>] [-s|--strip]");
            println!("Options:");
            println!("  -j, --json          print metadata as JSON");
            println!("  -o, --output <path> write the result to <path> instead of stdout");
            println!("  -s, --strip         strip metadata from the file (currently JPEG/PNG only)");
            println!("  -v, --version       print version information");
            println!("  -h, --help          print this help message");
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

            let mut parser = nom_exif::MediaParser::new();
            let metadata = extract::extract(&args.file, &mut parser)
                .map_err(|e| AppError::Runtime(e.to_string()))?;

            let format = if args.json {
                output::RenderFormat::Json
            } else {
                output::RenderFormat::Text
            };

            output::write_output(&metadata, format, args.output.as_deref())
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

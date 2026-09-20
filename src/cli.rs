use lexopt::{Arg, Parser};

pub struct Args {
    pub file: String,
    pub json: bool,
    pub output: Option<String>,
}

pub enum CliResult {
    Args(Args),
    Help,
    Version,
}

pub fn parse_args() -> Result<CliResult, lexopt::Error> {
    let mut parser = Parser::from_env();
    let mut file = None;
    let mut json = false;
    let mut output = None;

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Short('j') | Arg::Long("json") => {
                json = true;
            }
            Arg::Short('o') | Arg::Long("output") => {
                output = Some(parser.value()?.to_string_lossy().into_owned());
            }
            Arg::Short('v') | Arg::Long("version") => {
                return Ok(CliResult::Version);
            }
            Arg::Short('h') | Arg::Long("help") => {
                return Ok(CliResult::Help);
            }
            Arg::Value(val) if file.is_none() => {
                file = Some(val.to_string_lossy().into_owned());
            }
            _ => return Err(arg.unexpected()),
        }
    }

    match file {
        Some(f) => Ok(CliResult::Args(Args {
            file: f,
            json,
            output,
        })),
        None => Err(lexopt::Error::MissingValue {
            option: Some("file".to_string()),
        }),
    }
}

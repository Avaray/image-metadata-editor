use lexopt::{Arg, Parser};

pub struct Args {
    pub file: String,
    pub key: Option<String>,
    pub output: Option<String>,
    pub strip: bool,
    pub set: std::collections::BTreeMap<String, String>,
}

pub enum CliResult {
    Args(Args),
    Help,
    Version,
}

pub fn parse_args() -> Result<CliResult, lexopt::Error> {
    let mut parser = Parser::from_env();
    let mut file = None;
    let mut key = None;
    let mut output = None;
    let mut strip = false;
    let mut set = std::collections::BTreeMap::new();

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Short('s') | Arg::Long("strip") => {
                strip = true;
            }
            Arg::Long("set") => {
                let val = parser.value()?.to_string_lossy().into_owned();
                if let Some((k, v)) = val.split_once('=') {
                    set.insert(k.to_string(), v.to_string());
                } else {
                    return Err(lexopt::Error::Custom(
                        "Invalid --set format, expected Key=Value".into(),
                    ));
                }
            }
            Arg::Short('k') | Arg::Long("key") => {
                let raw = parser.value()?.to_string_lossy().into_owned();
                // Accept both ".Exif.Model" and "Exif.Model"
                key = Some(raw.trim_start_matches('.').to_string());
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
            key,
            output,
            strip,
            set,
        })),
        None => Err(lexopt::Error::MissingValue {
            option: Some("file".to_string()),
        }),
    }
}

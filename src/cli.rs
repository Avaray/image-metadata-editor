use lexopt::{Arg, Parser};

pub struct Args {
    pub file: String,
    pub key: Option<String>,
    pub output: Option<String>,
    pub strip: bool,
    pub set: std::collections::BTreeMap<String, String>,
    pub set_json: Option<String>,
    pub delete: Vec<String>,
    pub recursive: bool,
    pub directory: bool,
    pub tui: bool,
    pub power_user: bool,
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
    let mut set_json = None;
    let mut delete = Vec::new();
    let mut recursive = false;
    let mut directory = false;
    let mut tui = false;
    let mut power_user = false;

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Short('t') | Arg::Long("tui") => {
                tui = true;
            }
            Arg::Short('s') | Arg::Long("strip") => {
                strip = true;
            }
            Arg::Short('r') | Arg::Long("recursive") => {
                recursive = true;
            }
            Arg::Short('d') | Arg::Long("dir") | Arg::Long("directory") => {
                directory = true;
            }
            Arg::Short('p') | Arg::Long("power") => {
                power_user = true;
            }
            Arg::Long("set") => {
                let val = parser.value()?.to_string_lossy().into_owned();
                if let Some((k, v)) = val.split_once('=') {
                    set.insert(k.to_string(), v.to_string());
                } else {
                    return Err(lexopt::Error::Custom("Invalid --set format, expected Key=Value or .path.to.key=Value".into()));
                }
            }
            Arg::Long("set-json") => {
                set_json = Some(parser.value()?.to_string_lossy().into_owned());
            }
            Arg::Long("delete") => {
                delete.push(parser.value()?.to_string_lossy().into_owned());
            }
            Arg::Short('k') | Arg::Long("key") => {
                let raw = parser.value()?.to_string_lossy().into_owned();
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
        Some(f) => Ok(CliResult::Args(Args { file: f, key, output, strip, set, set_json, delete, recursive, directory, tui, power_user })),
        None => Err(lexopt::Error::MissingValue { option: Some("file".to_string()) }),
    }
}

use crate::model::Output;
use std::fs::File;
use std::io::{self, Write};

pub fn write_output(out: &Output, dest_path: Option<&str>) -> io::Result<()> {
    let mut buffer = Vec::new();

    serde_json::to_writer(&mut buffer, out)?;
    buffer.push(b'\n');

    if let Some(p) = dest_path {
        let mut f = File::create(p)?;
        f.write_all(&buffer)?;
    } else {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(&buffer)?;
    }

    Ok(())
}

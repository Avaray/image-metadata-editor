use crate::model::Output;
use std::fs::File;
use std::io::{self, Write};

pub enum RenderFormat {
    Text,
    Json,
}

pub fn write_output(out: &Output, format: RenderFormat, dest_path: Option<&str>) -> io::Result<()> {
    let mut buffer = Vec::new();

    match format {
        RenderFormat::Json => {
            serde_json::to_writer(&mut buffer, out)?;
            buffer.push(b'\n');
        }
        RenderFormat::Text => {
            use std::fmt::Write;
            let mut s = String::new();

            if out.is_empty() {
                writeln!(&mut s, "No metadata found.").unwrap();
            } else {
                for (dir, tags) in out {
                    writeln!(&mut s, "[{}]", dir).unwrap();
                    for (tag, val) in tags {
                        writeln!(&mut s, "  {}: {}", tag, val).unwrap();
                    }
                }
            }
            buffer.extend_from_slice(s.as_bytes());
        }
    }

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

# mex

`mex` (Metadata EXtractor) is a fast, lightweight, and portable CLI tool that reads metadata (like EXIF) from image, video, and audio files and prints it as readable text or JSON. 

## Usage

```bash
# Print metadata as readable text (default)
mex <file>

# Print metadata as JSON
mex <file> -j
mex <file> --json

# Write the result to a file instead of stdout
mex <file> -o <path>
mex <file> --output <path>

# Print version
mex -v
mex --version

# Print help
mex -h
mex --help
```

## Features
- **Fast**: Avoids loading the whole file into memory. It only reads the required bytes to extract metadata.
- **Standalone**: Shipped as a statically linked binary with no dependencies.
- **Formats supported**: JPEG, PNG, WebP, HEIC, AVIF, TIFF, CR3, RAF, IIQ, MP4, MOV, 3GP, MKV, WebM.

## Output Examples

### Text Output

```
File: sample.jpg
[Exif]
  Model: DSC-RX100M5A
  DateTimeOriginal: 2023:08:14 10:20:30
[IFD0]
  Make: SONY
```

### JSON Output

```json
{
  "file": "sample.jpg",
  "directories": {
    "Exif": {
      "Model": "DSC-RX100M5A",
      "DateTimeOriginal": "2023:08:14 10:20:30"
    },
    "IFD0": {
      "Make": "SONY"
    }
  }
}
```

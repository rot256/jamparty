use std::{env, fs, io, path::PathBuf};

use brotli::{BrotliCompress, enc::BrotliEncoderParams};

fn main() -> io::Result<()> {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let params = BrotliEncoderParams {
        quality: 11,
        lgwin: 24,
        ..BrotliEncoderParams::default()
    };

    for name in [
        "client.js",
        "pdf.js",
        "pdf-worker.js",
        "pdf.css",
        "pdf.html",
        "pdf-cmaps.bin",
    ] {
        let path = format!("assets/{name}");
        println!("cargo:rerun-if-changed={path}");
        let data = fs::read(path)?;
        let mut output = fs::File::create(out.join(format!("{name}.br")))?;
        BrotliCompress(&mut data.as_slice(), &mut output, &params)?;
    }
    Ok(())
}

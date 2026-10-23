use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=APP_VERSION");
    println!("cargo:rerun-if-changed=bin");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let mut generated = String::new();
    let extension = if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        ".exe"
    } else {
        ""
    };
    for (name, constant) in [("yt-dlp", "YTDLP"), ("ffmpeg", "FFMPEG"), ("deno", "DENO")] {
        let filename = format!("{name}{extension}");
        let source = PathBuf::from("bin").join(&filename);
        if source.is_file() {
            let destination = out.join(&filename);
            fs::copy(&source, &destination).expect("copy bundled tool");
            println!("cargo:rerun-if-changed={}", source.display());
            generated.push_str(&format!(
                "pub const {constant}: Option<&[u8]> = Some(include_bytes!({:?}));\n",
                destination
            ));
        } else {
            generated.push_str(&format!("pub const {constant}: Option<&[u8]> = None;\n"));
        }
    }
    let version = env::var("APP_VERSION")
        .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "dev".into()));
    generated.push_str(&format!("pub const APP_VERSION: &str = {:?};\n", version));
    fs::write(out.join("embedded_tools.rs"), generated).expect("write generated tool metadata");
}

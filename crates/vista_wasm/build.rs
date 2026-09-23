//! Minifies WGSL shaders at build time.
//!
//! Shader source is embedded in the WASM binary as text, so comments and
//! indentation cost real download size. This strips `//` comments, leading
//! indentation, and blank lines, and collapses runs of spaces. WGSL has no
//! string literals, so this is safe; `render::shaders` tests validate every
//! minified shader with naga.

use std::fs;
use std::path::Path;

fn minify(source: &str) -> String {
  let mut out = String::with_capacity(source.len() / 2);

  for line in source.lines() {
    let code = match line.find("//") {
      Some(index) => &line[..index],
      None => line,
    };
    let mut previous_space = false;
    let mut compact = String::with_capacity(code.len());

    for character in code.trim().chars() {
      if character == ' ' || character == '\t' {
        if !previous_space {
          compact.push(' ');
        }

        previous_space = true;
      } else {
        compact.push(character);
        previous_space = false;
      }
    }

    if !compact.is_empty() {
      out.push_str(&compact);
      out.push('\n');
    }
  }

  out
}

fn main() {
  let source_dir = Path::new("src/shaders");
  let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
  println!("cargo:rerun-if-changed=src/shaders");

  let Ok(entries) = fs::read_dir(source_dir) else {
    return;
  };

  for entry in entries.flatten() {
    let path = entry.path();

    if path.extension().and_then(|extension| extension.to_str()) != Some("wgsl") {
      continue;
    }

    println!("cargo:rerun-if-changed={}", path.display());

    if let (Ok(source), Some(name)) = (fs::read_to_string(&path), path.file_name()) {
      let _ = fs::write(Path::new(&out_dir).join(name), minify(&source));
    }
  }
}

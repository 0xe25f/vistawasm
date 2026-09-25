//! Minifies WGSL shaders at build time.
//!
//! Shader source is embedded in the WASM binary as text, so comments,
//! indentation and spacing cost real download size. This strips `//`
//! comments and drops every space and line break that does not separate
//! two words (identifiers, keywords or numbers) or two operator
//! characters, which could otherwise join into another token, such as
//! `- -` into `--`. WGSL has no string literals, so this is safe;
//! `render::shaders` tests validate every minified shader with naga.

use std::fs;
use std::path::Path;

fn is_word(character: char) -> bool {
  character.is_ascii_alphanumeric() || character == '_' || character == '.'
}

fn is_operator(character: char) -> bool {
  "+-*/%&|^!=<>".contains(character)
}

fn minify(source: &str) -> String {
  let mut out = String::with_capacity(source.len() / 2);
  // A space or line break seen since the last character written.
  let mut gap = false;

  for line in source.lines() {
    let code = match line.find("//") {
      Some(index) => &line[..index],
      None => line,
    };

    for character in code.chars().chain(std::iter::once('\n')) {
      if character.is_whitespace() {
        gap = !out.is_empty();
        continue;
      }

      if gap {
        let previous = out.chars().next_back().unwrap_or(' ');

        if (is_word(previous) && is_word(character))
          || (is_operator(previous) && is_operator(character))
        {
          out.push(' ');
        }

        gap = false;
      }

      out.push(character);
    }
  }

  out.push('\n');
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

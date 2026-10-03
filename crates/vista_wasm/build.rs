//! Minifies WGSL shaders at build time.
//!
//! Shader source is embedded in the WASM binary as text, so comments,
//! indentation, spacing and long names cost real download size. This
//! strips `//` comments and drops every space and line break that does
//! not separate two words (identifiers, keywords or numbers) or two
//! operator characters, which could otherwise join into another token,
//! such as `- -` into `--`. WGSL has no string literals, so this is safe.
//!
//! It then gives the names the shaders declare (functions, structs and
//! their members, constants, variables and parameters) short ones, from
//! one map for every shader, as the engine joins some of them into one
//! module at run time. Names the engine asks for (entry points and
//! `override` constants) keep theirs, as do WGSL's own words.
//! `render::shaders` tests validate every minified shader with naga and
//! check every entry point keeps its name.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// WGSL's keywords and type names: never renamed. A shader cannot
/// declare these, but `declared` reads any word before a `:`.
const KEYWORDS: &str = "alias array atomic bitcast bool break case const const_assert continue \
  continuing default diagnostic discard else enable f16 f32 false fn for i32 if let loop override \
  ptr requires return sampler sampler_comparison struct switch true u32 var vec2 vec3 vec4 while \
  mat2x2 mat2x3 mat2x4 mat3x2 mat3x3 mat3x4 mat4x2 mat4x3 mat4x4 function private workgroup \
  uniform storage read write read_write";

/// WGSL's built-in functions. A variable or member may share one's name,
/// so only uses of such a name that are not calls are renamed, and a
/// shader function by such a name keeps it.
const BUILTIN_FUNCTIONS: &str = "abs acos acosh all any arrayLength asin asinh atan atanh \
  atan2 ceil clamp cos cosh countLeadingZeros countOneBits countTrailingZeros cross degrees \
  determinant distance dot exp exp2 extractBits faceForward firstLeadingBit firstTrailingBit \
  floor fma fract frexp insertBits inverseSqrt ldexp length log log2 max min mix modf normalize \
  pow quantizeToF16 radians reflect refract reverseBits round saturate select sign sin sinh \
  smoothstep sqrt step tan tanh transpose trunc dpdx dpdxCoarse dpdxFine dpdy dpdyCoarse \
  dpdyFine fwidth fwidthCoarse fwidthFine textureDimensions textureGather textureGatherCompare \
  textureLoad textureNumLayers textureNumLevels textureNumSamples textureSample textureSampleBias \
  textureSampleCompare textureSampleCompareLevel textureSampleGrad textureSampleLevel \
  textureSampleBaseClampToEdge textureStore atomicLoad atomicStore atomicAdd atomicSub atomicMax \
  atomicMin atomicAnd atomicOr atomicXor atomicExchange atomicCompareExchangeWeak pack4x8snorm \
  pack4x8unorm pack2x16snorm pack2x16unorm pack2x16float unpack4x8snorm unpack4x8unorm \
  unpack2x16snorm unpack2x16unorm unpack2x16float storageBarrier workgroupBarrier textureBarrier \
  workgroupUniformLoad";

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

/// The identifiers of `source`, with the byte ranges they cover.
fn identifiers(source: &str) -> Vec<(usize, usize)> {
  let bytes = source.as_bytes();
  let mut found = Vec::new();
  let mut at = 0;

  while at < bytes.len() {
    let start = at;

    if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
      while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
        at += 1;
      }

      found.push((start, at));
    } else if bytes[at].is_ascii_digit() {
      // Numbers, with their suffixes and exponents, are not names.
      while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'.') {
        at += 1;
      }
    } else {
      at += 1;
    }
  }

  found
}

/// The names `source` declares: after `fn`, `struct`, `let`, `var`
/// (with or without an address space), `const`, `override` and `alias`,
/// and before a `:` (struct members and parameters).
fn declared(source: &str, names: &mut BTreeSet<String>) {
  let found = identifiers(source);

  for (index, &(start, end)) in found.iter().enumerate() {
    let word = &source[start..end];
    let after = source[end..].trim_start();

    if after.starts_with(':') && !after.starts_with("::") {
      names.insert(word.to_string());
    }

    if matches!(
      word,
      "fn" | "struct" | "let" | "const" | "override" | "alias" | "var"
    ) {
      let mut next = index + 1;

      // `var<storage, read_write> name`: skip the address space.
      if word == "var" && source[end..].trim_start().starts_with('<') {
        let close = end + source[end..].find('>').unwrap_or(0);
        while next < found.len() && found[next].0 < close {
          next += 1;
        }
      }

      if let Some(&(s, e)) = found.get(next) {
        names.insert(source[s..e].to_string());
      }
    }
  }
}

/// The names the engine asks for: entry points (the function after a
/// `@vertex`, `@fragment` or `@compute` attribute) and `override`
/// constants, which pipelines set by name.
fn named_by_engine(source: &str, names: &mut BTreeSet<String>) {
  let found = identifiers(source);

  for (index, &(start, end)) in found.iter().enumerate() {
    let word = &source[start..end];
    let attribute = source[..start].ends_with('@');

    if (attribute && matches!(word, "vertex" | "fragment" | "compute")) || word == "override" {
      // The next `fn` after a stage attribute, or the name after
      // `override`.
      let rest = &found[index + 1..];
      let name = if word == "override" {
        rest.first()
      } else {
        rest
          .iter()
          .position(|&(s, e)| &source[s..e] == "fn")
          .and_then(|at| rest.get(at + 1))
      };

      if let Some(&(s, e)) = name {
        names.insert(source[s..e].to_string());
      }
    }
  }
}

/// Whether `name` could be a vector swizzle, which member access would
/// confuse with a struct member of that name.
fn is_swizzle(name: &str) -> bool {
  name.len() <= 4
    && (name.chars().all(|c| "xyzw".contains(c)) || name.chars().all(|c| "rgba".contains(c)))
}

/// Short names, shortest first: an upper-case letter, then letters and
/// digits. WGSL's own words are all lower case or longer.
fn short_names() -> impl Iterator<Item = String> {
  const FIRST: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
  const REST: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let one = FIRST.chars().map(String::from);
  let two = FIRST
    .chars()
    .flat_map(|a| REST.chars().map(move |b| format!("{a}{b}")));
  let three = FIRST.chars().flat_map(|a| {
    REST
      .chars()
      .flat_map(move |b| REST.chars().map(move |c| format!("{a}{b}{c}")))
  });
  one.chain(two).chain(three)
}

/// A short name for every renamable name the shaders declare, the most
/// used bytes first.
fn rename_map(shaders: &[(String, String)], keep: &BTreeSet<String>) -> BTreeMap<String, String> {
  let mut names = BTreeSet::new();
  let mut uses: BTreeMap<String, usize> = BTreeMap::new();
  let mut every = BTreeSet::new();

  for (_, source) in shaders {
    declared(source, &mut names);

    for (start, end) in identifiers(source) {
      *uses.entry(source[start..end].to_string()).or_default() += 1;
      every.insert(source[start..end].to_string());
    }
  }

  let mut renamed: Vec<&String> = names
    .iter()
    .filter(|name| !keep.contains(*name) && !is_swizzle(name))
    .collect();
  // A shader function named after a built-in would be hard to tell from
  // it; none is, so this only guards.
  renamed.retain(|name| {
    !(builtin_function(name)
      && shaders
        .iter()
        .any(|(_, s)| s.contains(&format!("fn {name}("))))
  });
  renamed.sort_by_key(|name| {
    (
      std::cmp::Reverse(uses.get(*name).copied().unwrap_or(0) * name.len()),
      *name,
    )
  });
  let mut fresh = short_names().filter(|name| !every.contains(name));
  let mut map = BTreeMap::new();

  for name in renamed {
    if let Some(short) = fresh.next() {
      if short.len() < name.len() {
        map.insert(name.clone(), short);
      }
    }
  }

  map
}

fn builtin_function(name: &str) -> bool {
  BUILTIN_FUNCTIONS
    .split_whitespace()
    .any(|builtin| builtin == name)
}

/// `source` with every renamable use of a name in `map` renamed. Words
/// right after an `@` are attribute names, and words inside `@builtin(…)`
/// and `@interpolate(…)` are WGSL's own, so they stay; so do calls to a
/// built-in function that a variable shares its name with.
fn rename(source: &str, map: &BTreeMap<String, String>) -> String {
  let mut out = String::with_capacity(source.len());
  let mut last = 0;

  for (start, end) in identifiers(source) {
    let word = &source[start..end];
    let before = &source[..start];
    let in_attribute = before.ends_with('@')
      || [
        "@builtin(",
        "@interpolate(",
        "@interpolate(flat,",
        "@interpolate(linear,",
        "@interpolate(perspective,",
      ]
      .iter()
      .any(|attribute| before.ends_with(attribute));
    let call = source[end..].starts_with('(');

    if in_attribute || (call && builtin_function(word)) {
      continue;
    }

    if let Some(short) = map.get(word) {
      out.push_str(&source[last..start]);
      out.push_str(short);
      last = end;
    }
  }

  out.push_str(&source[last..]);
  out
}

fn main() {
  let source_dir = Path::new("src/shaders");
  let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
  println!("cargo:rerun-if-changed=src/shaders");

  let Ok(entries) = fs::read_dir(source_dir) else {
    return;
  };

  let mut shaders = Vec::new();

  for entry in entries.flatten() {
    let path = entry.path();

    if path.extension().and_then(|extension| extension.to_str()) != Some("wgsl") {
      continue;
    }

    println!("cargo:rerun-if-changed={}", path.display());

    if let (Ok(source), Some(name)) = (fs::read_to_string(&path), path.file_name()) {
      shaders.push((name.to_string_lossy().into_owned(), minify(&source)));
    }
  }

  // Sorted, so the names given do not depend on the directory's order.
  shaders.sort();
  let mut keep: BTreeSet<String> = KEYWORDS.split_whitespace().map(String::from).collect();

  for (_, source) in &shaders {
    named_by_engine(source, &mut keep);
  }

  let map = rename_map(&shaders, &keep);

  for (name, source) in &shaders {
    let _ = fs::write(Path::new(&out_dir).join(name), rename(source, &map));
  }

  // The map, for tests that look a shader's variable up by name.
  let table: String = map
    .iter()
    .map(|(from, to)| format!("  (\"{from}\", \"{to}\"),\n"))
    .collect();
  let _ = fs::write(
    Path::new(&out_dir).join("shader_names.rs"),
    format!("&[\n{table}]\n"),
  );
}

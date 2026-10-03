//! The option keys the engine reads, for the wrapper's unknown-key check.
//!
//! The engine reads each option struct's fields by name and never sees
//! other keys, so the wrapper compares the options it is given with
//! `js/src/option-keys.ts` and warns about the rest. That file is
//! generated from the types here: this test walks every option struct
//! with a deserialiser that records the fields each one asks for, and
//! fails when the file differs. Regenerate it with
//!
//! ```sh
//! UPDATE_OPTION_KEYS=1 cargo test -p vista_wasm --test option_keys
//! ```

use serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};
use serde::Deserialize;
use vista_types::{DemLoadOptions, FractalTerrainOptions, RawHeightmapOptions, VistaEngineOptions};

/// What one value accepts: the keys of an object, an array of such
/// values, or anything (a number, a string, a map of names).
#[derive(Debug, Default)]
enum Shape {
  #[default]
  Any,
  Object(Vec<(String, Shape)>),
  List(Box<Shape>),
}

#[derive(Debug)]
struct Error(String);

impl std::fmt::Display for Error {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter.write_str(&self.0)
  }
}

impl std::error::Error for Error {}

impl de::Error for Error {
  fn custom<T: std::fmt::Display>(message: T) -> Self {
    Self(message.to_string())
  }
}

/// A deserialiser that hands every type a plausible value and records the
/// shape it asked for in `shape`.
struct Probe<'a> {
  shape: &'a mut Shape,
}

impl<'de> de::Deserializer<'de> for Probe<'_> {
  type Error = Error;

  /// Untyped values (a string or a list, say) are probed as a list, so a
  /// list of objects shows its keys.
  fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    self.deserialize_seq(visitor)
  }

  fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_bool(false)
  }

  fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_i64(0)
  }

  fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_i64(0)
  }

  fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_i64(0)
  }

  fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_i64(0)
  }

  fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_u64(0)
  }

  fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_u64(0)
  }

  fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_u64(0)
  }

  fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_u64(0)
  }

  fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_f64(0.0)
  }

  fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_f64(0.0)
  }

  fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_char('a')
  }

  fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_str("")
  }

  fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_str("")
  }

  fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_bytes(&[])
  }

  fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_bytes(&[])
  }

  fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_some(self)
  }

  fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_unit()
  }

  fn deserialize_unit_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    visitor: V,
  ) -> Result<V::Value, Error> {
    visitor.visit_unit()
  }

  fn deserialize_newtype_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    visitor: V,
  ) -> Result<V::Value, Error> {
    visitor.visit_newtype_struct(self)
  }

  fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    *self.shape = Shape::List(Box::default());
    let Shape::List(item) = self.shape else {
      unreachable!()
    };
    // Twelve items: the longest list a type insists on (material tints).
    visitor.visit_seq(Items { item, left: 12 })
  }

  fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
    // Fixed-size arrays of numbers (colours, positions): any value.
    visitor.visit_seq(Items {
      item: &mut Shape::Any,
      left: len,
    })
  }

  fn deserialize_tuple_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    len: usize,
    visitor: V,
  ) -> Result<V::Value, Error> {
    self.deserialize_tuple(len, visitor)
  }

  /// Maps of names (weather presets, `next` weights) take any key; the
  /// engine checks what is inside them itself.
  fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_map(de::value::MapDeserializer::new(std::iter::empty::<(
      &str,
      &str,
    )>()))
  }

  fn deserialize_struct<V: Visitor<'de>>(
    self,
    _name: &'static str,
    fields: &'static [&'static str],
    visitor: V,
  ) -> Result<V::Value, Error> {
    *self.shape = Shape::Object(
      fields
        .iter()
        .map(|field| (field.to_string(), Shape::Any))
        .collect(),
    );
    let Shape::Object(entries) = self.shape else {
      unreachable!()
    };
    visitor.visit_map(Fields { entries, next: 0 })
  }

  fn deserialize_enum<V: Visitor<'de>>(
    self,
    _name: &'static str,
    variants: &'static [&'static str],
    visitor: V,
  ) -> Result<V::Value, Error> {
    visitor.visit_enum(variants[0].into_deserializer())
  }

  fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_str("")
  }

  fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
    visitor.visit_unit()
  }
}

/// Every field of a struct, each probed into its entry.
struct Fields<'a> {
  entries: &'a mut Vec<(String, Shape)>,
  next: usize,
}

impl<'de> de::MapAccess<'de> for Fields<'_> {
  type Error = Error;

  fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
    match self.entries.get(self.next) {
      Some((name, _)) => seed
        .deserialize(name.as_str().into_deserializer())
        .map(Some),
      None => Ok(None),
    }
  }

  fn next_value_seed<S: DeserializeSeed<'de>>(&mut self, seed: S) -> Result<S::Value, Error> {
    let shape = &mut self.entries[self.next].1;
    self.next += 1;
    seed.deserialize(Probe { shape })
  }
}

/// `left` items of a list, each probed into `item`.
struct Items<'a> {
  item: &'a mut Shape,
  left: usize,
}

impl<'de> de::SeqAccess<'de> for Items<'_> {
  type Error = Error;

  fn next_element_seed<S: DeserializeSeed<'de>>(
    &mut self,
    seed: S,
  ) -> Result<Option<S::Value>, Error> {
    if self.left == 0 {
      return Ok(None);
    }

    self.left -= 1;
    seed.deserialize(Probe { shape: self.item }).map(Some)
  }
}

fn shape_of<T: for<'de> Deserialize<'de>>() -> Shape {
  let mut shape = Shape::Any;
  T::deserialize(Probe { shape: &mut shape })
    .unwrap_or_else(|error| panic!("could not probe {}: {error}", std::any::type_name::<T>()));
  shape
}

/// The shape as a TypeScript literal: an object of keys, `0` for any
/// value, `[shape]` for a list.
fn literal(shape: &Shape, indent: usize, out: &mut String) {
  match shape {
    Shape::Any => out.push('0'),
    Shape::List(item) => match **item {
      // A list of plain values takes anything.
      Shape::Any => out.push('0'),
      _ => {
        out.push('[');
        literal(item, indent, out);
        out.push(']');
      }
    },
    Shape::Object(entries) => {
      out.push_str("{\n");

      for (index, (name, value)) in entries.iter().enumerate() {
        out.push_str(&"  ".repeat(indent + 1));
        out.push_str(name);
        out.push_str(": ");
        literal(value, indent + 1, out);
        out.push_str(if index + 1 < entries.len() {
          ",\n"
        } else {
          "\n"
        });
      }

      out.push_str(&"  ".repeat(indent));
      out.push('}');
    }
  }
}

fn generated() -> String {
  let roots = [
    ("engine", shape_of::<VistaEngineOptions>()),
    ("terrain", shape_of::<FractalTerrainOptions>()),
    ("raw", shape_of::<RawHeightmapOptions>()),
    ("dem", shape_of::<DemLoadOptions>()),
  ];
  let mut out = [
    "// Generated by `UPDATE_OPTION_KEYS=1 cargo test -p vista_wasm --test option_keys`",
    "// from the option types in `crates/vista_types`; do not edit by hand.",
    "//",
    "// The keys the engine reads, for the wrapper's unknown-key warnings: an",
    "// object of keys, `0` for any value, `[shape]` for a list of objects.",
    "// `engine` holds every setter's options under their names.",
    "",
    "import type { OptionKeys } from \"./errors.js\";",
    "",
    "export const OPTION_KEYS = ",
  ]
  .join("\n");
  literal(
    &Shape::Object(
      roots
        .into_iter()
        .map(|(name, shape)| (name.to_string(), shape))
        .collect(),
    ),
    0,
    &mut out,
  );
  out.push_str(" satisfies OptionKeys;\n");
  out
}

#[test]
fn the_wrappers_option_keys_match_the_engines() {
  let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../js/src/option-keys.ts");
  let expected = generated();

  if std::env::var_os("UPDATE_OPTION_KEYS").is_some() {
    std::fs::write(path, &expected).unwrap();
    return;
  }

  let actual = std::fs::read_to_string(path).unwrap_or_default();
  assert!(
    actual == expected,
    "js/src/option-keys.ts is out of date with the option types. Regenerate it with {}.",
    "`UPDATE_OPTION_KEYS=1 cargo test -p vista_wasm --test option_keys`"
  );
}

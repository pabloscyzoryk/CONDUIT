//! Lossless serde value tree. Floating-point bits and non-string map keys survive JSON.
use serde::{
    de::{self, DeserializeOwned, IntoDeserializer},
    ser, Deserialize, Serialize,
};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Exact {
    Unit,
    Bool(bool),
    I(i64),
    U(u64),
    F32(u32),
    F64(u64),
    String(String),
    Bytes(Vec<u8>),
    Some(Box<Exact>),
    None,
    Seq(Vec<Exact>),
    Map(Vec<(Exact, Exact)>),
    Enum(String, Box<Exact>),
}
impl Exact {
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::String(v) => v.len(),
                Self::Bytes(v) => v.len(),
                Self::Some(v) => v.estimated_bytes(),
                Self::Enum(n, v) => n.len() + v.estimated_bytes(),
                Self::Seq(v) => v.iter().map(Self::estimated_bytes).sum(),
                Self::Map(v) => v
                    .iter()
                    .map(|(k, v)| k.estimated_bytes() + v.estimated_bytes())
                    .sum(),
                _ => 0,
            }
    }
}
#[derive(Debug)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
impl ser::Error for Error {
    fn custom<T: fmt::Display>(m: T) -> Self {
        Self(m.to_string())
    }
}
impl de::Error for Error {
    fn custom<T: fmt::Display>(m: T) -> Self {
        Self(m.to_string())
    }
}
type Result<T> = std::result::Result<T, Error>;
pub fn encode<T: Serialize + ?Sized>(v: &T) -> Result<Exact> {
    v.serialize(Encoder)
}
pub fn decode<T: DeserializeOwned>(v: &Exact) -> Result<T> {
    T::deserialize(v)
}

struct Encoder;
struct Seq {
    values: Vec<Exact>,
    variant: Option<String>,
}
struct Map {
    values: Vec<(Exact, Exact)>,
    key: Option<Exact>,
    variant: Option<String>,
    sort: bool,
}
impl ser::Serializer for Encoder {
    type Ok = Exact;
    type Error = Error;
    type SerializeSeq = Seq;
    type SerializeTuple = Seq;
    type SerializeTupleStruct = Seq;
    type SerializeTupleVariant = Seq;
    type SerializeMap = Map;
    type SerializeStruct = Map;
    type SerializeStructVariant = Map;
    fn serialize_bool(self, v: bool) -> Result<Exact> {
        Ok(Exact::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Exact> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i16(self, v: i16) -> Result<Exact> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i32(self, v: i32) -> Result<Exact> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i64(self, v: i64) -> Result<Exact> {
        Ok(Exact::I(v))
    }
    fn serialize_u8(self, v: u8) -> Result<Exact> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u16(self, v: u16) -> Result<Exact> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u32(self, v: u32) -> Result<Exact> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u64(self, v: u64) -> Result<Exact> {
        Ok(Exact::U(v))
    }
    fn serialize_f32(self, v: f32) -> Result<Exact> {
        Ok(Exact::F32(v.to_bits()))
    }
    fn serialize_f64(self, v: f64) -> Result<Exact> {
        Ok(Exact::F64(v.to_bits()))
    }
    fn serialize_char(self, v: char) -> Result<Exact> {
        self.serialize_str(&v.to_string())
    }
    fn serialize_str(self, v: &str) -> Result<Exact> {
        Ok(Exact::String(v.to_owned()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Exact> {
        Ok(Exact::Bytes(v.to_vec()))
    }
    fn serialize_none(self) -> Result<Exact> {
        Ok(Exact::None)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<Exact> {
        Ok(Exact::Some(Box::new(encode(v)?)))
    }
    fn serialize_unit(self) -> Result<Exact> {
        Ok(Exact::Unit)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Exact> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, v: &'static str) -> Result<Exact> {
        Ok(Exact::Enum(v.into(), Box::new(Exact::Unit)))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<Exact> {
        encode(v)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        n: &'static str,
        v: &T,
    ) -> Result<Exact> {
        Ok(Exact::Enum(n.into(), Box::new(encode(v)?)))
    }
    fn serialize_seq(self, n: Option<usize>) -> Result<Seq> {
        Ok(Seq {
            values: Vec::with_capacity(n.unwrap_or(0)),
            variant: None,
        })
    }
    fn serialize_tuple(self, n: usize) -> Result<Seq> {
        self.serialize_seq(Some(n))
    }
    fn serialize_tuple_struct(self, _: &'static str, n: usize) -> Result<Seq> {
        self.serialize_tuple(n)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        v: &'static str,
        n: usize,
    ) -> Result<Seq> {
        Ok(Seq {
            values: Vec::with_capacity(n),
            variant: Some(v.into()),
        })
    }
    fn serialize_map(self, n: Option<usize>) -> Result<Map> {
        Ok(Map {
            values: Vec::with_capacity(n.unwrap_or(0)),
            key: None,
            variant: None,
            sort: true,
        })
    }
    fn serialize_struct(self, _: &'static str, n: usize) -> Result<Map> {
        Ok(Map {
            values: Vec::with_capacity(n),
            key: None,
            variant: None,
            sort: false,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        v: &'static str,
        n: usize,
    ) -> Result<Map> {
        Ok(Map {
            values: Vec::with_capacity(n),
            key: None,
            variant: Some(v.into()),
            sort: false,
        })
    }
}
impl Seq {
    fn add<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        self.values.push(encode(v)?);
        Ok(())
    }
    fn finish(self) -> Result<Exact> {
        let v = Exact::Seq(self.values);
        Ok(match self.variant {
            Some(n) => Exact::Enum(n, Box::new(v)),
            None => v,
        })
    }
}
macro_rules! seq_impl {
    ($trait:ident,$method:ident) => {
        impl ser::$trait for Seq {
            type Ok = Exact;
            type Error = Error;
            fn $method<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
                self.add(v)
            }
            fn end(self) -> Result<Exact> {
                self.finish()
            }
        }
    };
}
seq_impl!(SerializeSeq, serialize_element);
seq_impl!(SerializeTuple, serialize_element);
seq_impl!(SerializeTupleStruct, serialize_field);
seq_impl!(SerializeTupleVariant, serialize_field);
impl Map {
    fn field<T: Serialize + ?Sized>(&mut self, k: &'static str, v: &T) -> Result<()> {
        self.values.push((Exact::String(k.into()), encode(v)?));
        Ok(())
    }
    fn finish(mut self) -> Result<Exact> {
        // Canonical map order is independent of process-random HashMap seeds.
        if self.sort {
            self.values
                .sort_by_cached_key(|(key, _)| serde_json::to_string(key).unwrap_or_default());
        }
        let v = Exact::Map(self.values);
        Ok(match self.variant {
            Some(n) => Exact::Enum(n, Box::new(v)),
            None => v,
        })
    }
}
impl ser::SerializeMap for Map {
    type Ok = Exact;
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<()> {
        self.key = Some(encode(k)?);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        self.values.push((
            self.key
                .take()
                .ok_or_else(|| Error("map value without key".into()))?,
            encode(v)?,
        ));
        Ok(())
    }
    fn end(self) -> Result<Exact> {
        self.finish()
    }
}
impl ser::SerializeStruct for Map {
    type Ok = Exact;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, k: &'static str, v: &T) -> Result<()> {
        self.field(k, v)
    }
    fn end(self) -> Result<Exact> {
        self.finish()
    }
}
impl ser::SerializeStructVariant for Map {
    type Ok = Exact;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, k: &'static str, v: &T) -> Result<()> {
        self.field(k, v)
    }
    fn end(self) -> Result<Exact> {
        self.finish()
    }
}

struct SeqAccess<'a>(std::slice::Iter<'a, Exact>);
impl<'de> de::SeqAccess<'de> for SeqAccess<'de> {
    type Error = Error;
    fn next_element_seed<T: de::DeserializeSeed<'de>>(&mut self, s: T) -> Result<Option<T::Value>> {
        self.0.next().map(|v| s.deserialize(v)).transpose()
    }
}
struct MapAccess<'a> {
    iter: std::slice::Iter<'a, (Exact, Exact)>,
    value: Option<&'a Exact>,
}
impl<'de> de::MapAccess<'de> for MapAccess<'de> {
    type Error = Error;
    fn next_key_seed<T: de::DeserializeSeed<'de>>(&mut self, s: T) -> Result<Option<T::Value>> {
        match self.iter.next() {
            Some((k, v)) => {
                self.value = Some(v);
                s.deserialize(k).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<T: de::DeserializeSeed<'de>>(&mut self, s: T) -> Result<T::Value> {
        s.deserialize(
            self.value
                .take()
                .ok_or_else(|| Error("map value absent".into()))?,
        )
    }
}
struct EnumAccess<'a>(&'a str, &'a Exact);
impl<'de> de::EnumAccess<'de> for EnumAccess<'de> {
    type Error = Error;
    type Variant = &'de Exact;
    fn variant_seed<T: de::DeserializeSeed<'de>>(self, s: T) -> Result<(T::Value, Self::Variant)> {
        Ok((s.deserialize(self.0.into_deserializer())?, self.1))
    }
}
impl<'de> de::VariantAccess<'de> for &'de Exact {
    type Error = Error;
    fn unit_variant(self) -> Result<()> {
        if matches!(self, Exact::Unit) {
            Ok(())
        } else {
            Err(Error("expected unit variant".into()))
        }
    }
    fn newtype_variant_seed<T: de::DeserializeSeed<'de>>(self, s: T) -> Result<T::Value> {
        s.deserialize(self)
    }
    fn tuple_variant<V: de::Visitor<'de>>(self, _: usize, v: V) -> Result<V::Value> {
        de::Deserializer::deserialize_any(self, v)
    }
    fn struct_variant<V: de::Visitor<'de>>(
        self,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        de::Deserializer::deserialize_any(self, v)
    }
}
impl<'de> de::Deserializer<'de> for &'de Exact {
    type Error = Error;
    fn deserialize_any<V: de::Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match self {
            Exact::Unit => v.visit_unit(),
            Exact::Bool(x) => v.visit_bool(*x),
            Exact::I(x) => v.visit_i64(*x),
            Exact::U(x) => v.visit_u64(*x),
            Exact::F32(x) => v.visit_f32(f32::from_bits(*x)),
            Exact::F64(x) => v.visit_f64(f64::from_bits(*x)),
            Exact::String(x) => v.visit_borrowed_str(x),
            Exact::Bytes(x) => v.visit_borrowed_bytes(x),
            Exact::None => v.visit_none(),
            Exact::Some(x) => v.visit_some(x.as_ref()),
            Exact::Seq(x) => v.visit_seq(SeqAccess(x.iter())),
            Exact::Map(x) => v.visit_map(MapAccess {
                iter: x.iter(),
                value: None,
            }),
            Exact::Enum(n, x) => v.visit_enum(EnumAccess(n, x)),
        }
    }
    fn deserialize_option<V: de::Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match self {
            Exact::None => v.visit_none(),
            Exact::Some(x) => v.visit_some(x.as_ref()),
            _ => Err(Error("expected explicit option".into())),
        }
    }
    fn deserialize_newtype_struct<V: de::Visitor<'de>>(
        self,
        _: &'static str,
        v: V,
    ) -> Result<V::Value> {
        v.visit_newtype_struct(self)
    }
    serde::forward_to_deserialize_any! {bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct enum identifier ignored_any}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floating_bits_and_non_string_keys_survive_disk_json() {
        let values = [
            f64::from_bits(0x3ff0000000000001),
            -0.0,
            f64::INFINITY,
            f64::from_bits(0x7ff8000000000042),
        ];
        let input = std::collections::HashMap::from([((7i64, Some(4i64)), values.to_vec())]);
        let wire = encode(&input).unwrap();
        let json = serde_json::to_vec(&wire).unwrap();
        let restored: std::collections::HashMap<(i64, Option<i64>), Vec<f64>> =
            decode(&serde_json::from_slice::<Exact>(&json).unwrap()).unwrap();
        assert_eq!(
            restored[&(7, Some(4))]
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
            values.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
    }
    #[test]
    fn enum_and_option_distinguish_none_from_nan() {
        let x: Result<Option<f64>> = Ok(Some(f64::NAN));
        let y: std::result::Result<Option<f64>, String> = Ok(Some(f64::NAN));
        let back: std::result::Result<Option<f64>, String> = decode(&encode(&y).unwrap()).unwrap();
        assert!(back.unwrap().unwrap().is_nan());
        assert!(x.unwrap().unwrap().is_nan());
    }
}

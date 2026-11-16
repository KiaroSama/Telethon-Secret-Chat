//! Typed official Grammers TL values converted to constructor-tagged export data.
use serde::{
    Serialize,
    ser::{
        self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
        SerializeTupleStruct, SerializeTupleVariant,
    },
};
use serde_json::{Map, Value};
pub fn to_value<T: Serialize>(value: &T) -> std::result::Result<Value, serde_json::Error> {
    value.serialize(TlSerializer)
}
struct TlSerializer;
fn constructor(name: &str, variant: &str) -> String {
    super::tl_names::canonical_name(
        super::tl_names::unit_constructor(name, variant).unwrap_or(variant),
    )
    .into()
}
impl ser::Serializer for TlSerializer {
    type Ok = Value;
    type Error = serde_json::Error;
    type SerializeSeq = Sequence;
    type SerializeTuple = Sequence;
    type SerializeTupleStruct = Sequence;
    type SerializeTupleVariant = Sequence;
    type SerializeMap = Object;
    type SerializeStruct = Object;
    type SerializeStructVariant = Object;
    fn serialize_bool(self, v: bool) -> Result<Value, Self::Error> {
        Ok(Value::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Value, Self::Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i16(self, v: i16) -> Result<Value, Self::Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i32(self, v: i32) -> Result<Value, Self::Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i64(self, v: i64) -> Result<Value, Self::Error> {
        Ok(Value::from(v))
    }
    fn serialize_u8(self, v: u8) -> Result<Value, Self::Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u16(self, v: u16) -> Result<Value, Self::Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u32(self, v: u32) -> Result<Value, Self::Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u64(self, v: u64) -> Result<Value, Self::Error> {
        Ok(Value::from(v))
    }
    fn serialize_f32(self, v: f32) -> Result<Value, Self::Error> {
        self.serialize_f64(f64::from(v))
    }
    fn serialize_f64(self, v: f64) -> Result<Value, Self::Error> {
        Ok(Value::from(v))
    }
    fn serialize_char(self, v: char) -> Result<Value, Self::Error> {
        Ok(Value::String(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<Value, Self::Error> {
        Ok(Value::String(v.into()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Value, Self::Error> {
        Ok(Value::Array(v.iter().copied().map(Value::from).collect()))
    }
    fn serialize_none(self) -> Result<Value, Self::Error> {
        Ok(Value::Null)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<Value, Self::Error> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<Value, Self::Error> {
        Ok(Value::Null)
    }
    fn serialize_unit_struct(self, name: &'static str) -> Result<Value, Self::Error> {
        Ok(serde_json::json!({"_":super::tl_names::canonical_name(name)}))
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Value, Self::Error> {
        Ok(serde_json::json!({"_":constructor(name,variant)}))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        v: &T,
    ) -> Result<Value, Self::Error> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        v: &T,
    ) -> Result<Value, Self::Error> {
        v.serialize(self)
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Sequence, Self::Error> {
        Ok(Sequence(Vec::with_capacity(len.unwrap_or(0))))
    }
    fn serialize_tuple(self, len: usize) -> Result<Sequence, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<Sequence, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        len: usize,
    ) -> Result<Sequence, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Object, Self::Error> {
        Ok(Object {
            map: Map::new(),
            key: None,
        })
    }
    fn serialize_struct(self, name: &'static str, _len: usize) -> Result<Object, Self::Error> {
        Ok(Object {
            map: Map::from_iter([(
                "_".into(),
                Value::String(super::tl_names::canonical_name(name).into()),
            )]),
            key: None,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Object, Self::Error> {
        self.serialize_struct(variant, len)
    }
}
struct Sequence(Vec<Value>);
impl SerializeSeq for Sequence {
    type Ok = Value;
    type Error = serde_json::Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.0.push(v.serialize(TlSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value, Self::Error> {
        Ok(Value::Array(self.0))
    }
}
impl SerializeTuple for Sequence {
    type Ok = Value;
    type Error = serde_json::Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Value, Self::Error> {
        SerializeSeq::end(self)
    }
}
impl SerializeTupleStruct for Sequence {
    type Ok = Value;
    type Error = serde_json::Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Value, Self::Error> {
        SerializeSeq::end(self)
    }
}
impl SerializeTupleVariant for Sequence {
    type Ok = Value;
    type Error = serde_json::Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Value, Self::Error> {
        SerializeSeq::end(self)
    }
}
struct Object {
    map: Map<String, Value>,
    key: Option<String>,
}
impl SerializeMap for Object {
    type Ok = Value;
    type Error = serde_json::Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.key = Some(match v.serialize(TlSerializer)? {
            Value::String(s) => s,
            other => other.to_string(),
        });
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Self::Error> {
        let key = self
            .key
            .take()
            .ok_or_else(|| <Self::Error as ser::Error>::custom("Invalid TL map"))?;
        self.map.insert(key, v.serialize(TlSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value, Self::Error> {
        Ok(Value::Object(self.map))
    }
}
impl SerializeStruct for Object {
    type Ok = Value;
    type Error = serde_json::Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        self.map.insert(key.into(), v.serialize(TlSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value, Self::Error> {
        Ok(Value::Object(self.map))
    }
}
impl SerializeStructVariant for Object {
    type Ok = Value;
    type Error = serde_json::Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        SerializeStruct::serialize_field(self, key, v)
    }
    fn end(self) -> Result<Value, Self::Error> {
        SerializeStruct::end(self)
    }
}
/// Accepted request inputs are a small closed set of official constructor families.
pub fn request_args(value: &Value) -> Value {
    match value {
        Value::Array(a) => Value::Array(a.iter().map(request_args).collect()),
        Value::Object(object) => {
            let tag = object.get("_").and_then(Value::as_str).unwrap_or("");
            let family = [
                "InputPeer",
                "InputUser",
                "InputChannel",
                "InputMessagesFilter",
                "InputMessage",
                "InputDocumentFileLocation",
                "InputPhotoFileLocation",
                "InputTakeoutFileLocation",
            ]
            .iter()
            .find(|prefix| tag.starts_with(**prefix));
            let values = object
                .iter()
                .filter(|(key, _)| key.as_str() != "_")
                .map(|(key, v)| (key.clone(), request_args(v)))
                .collect::<Map<_, _>>();
            if let Some(family) = family {
                let variant = match tag {
                    "InputPeerSelf" => "PeerSelf",
                    "InputUserSelf" => "UserSelf",
                    "InputUser" => "User",
                    "InputChannel" => "Channel",
                    "InputMessageID" => "Id",
                    "InputDocumentFileLocation" => "InputDocumentFileLocation",
                    "InputPhotoFileLocation" => "InputPhotoFileLocation",
                    "InputTakeoutFileLocation" => "InputTakeoutFileLocation",
                    "InputMessagesFilterEmpty" => "InputMessagesFilterEmpty",
                    _ => tag.strip_prefix(family).unwrap_or(tag),
                };
                if values.is_empty() {
                    Value::String(variant.into())
                } else {
                    Value::Object(Map::from_iter([(variant.into(), Value::Object(values))]))
                }
            } else {
                Value::Object(values)
            }
        }
        other => other.clone(),
    }
}

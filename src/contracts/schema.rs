use regex::Regex;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug)]
pub(crate) enum StrictJsonError {
    DuplicateMember,
    Invalid,
}

pub(crate) struct StrictValue(pub(crate) Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

pub(crate) struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a finite JSON value without duplicate members")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("non_finite_number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(StrictValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some((key, StrictValue(value))) = map.next_entry::<String, StrictValue>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate_member"));
            }
            values.insert(key, value);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

pub(crate) fn strict_value(bytes: &[u8]) -> Result<Value, StrictJsonError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    match StrictValue::deserialize(&mut deserializer) {
        Ok(StrictValue(value)) if deserializer.end().is_ok() => Ok(value),
        Ok(_) => Err(StrictJsonError::Invalid),
        Err(error) if error.to_string().starts_with("duplicate_member") => {
            Err(StrictJsonError::DuplicateMember)
        }
        Err(_) => Err(StrictJsonError::Invalid),
    }
}

pub(crate) type SchemaRegistry = BTreeMap<String, Value>;

pub(crate) fn resolve_reference<'a>(
    reference: &str,
    owner_id: &str,
    registry: &'a SchemaRegistry,
) -> Option<(&'a Value, String)> {
    let (base, fragment) = reference
        .split_once('#')
        .map_or((reference, ""), |(base, fragment)| (base, fragment));
    let target_id = if base.is_empty() { owner_id } else { base };
    let mut target = registry.get(target_id)?;
    if !fragment.is_empty() {
        if !fragment.starts_with('/') {
            return None;
        }
        for token in fragment[1..].split('/') {
            let token = token.replace("~1", "/").replace("~0", "~");
            target = target.as_object()?.get(&token)?;
        }
    }
    Some((target, target_id.to_owned()))
}

pub(crate) fn validate_schema(
    value: &Value,
    schema: &Value,
    owner_id: &str,
    registry: &SchemaRegistry,
) -> bool {
    let Some(schema) = schema.as_object() else {
        return false;
    };
    if let Some(reference) = schema.get("$ref") {
        let Some(reference) = reference.as_str() else {
            return false;
        };
        let Some((target, target_id)) = resolve_reference(reference, owner_id, registry) else {
            return false;
        };
        return validate_schema(value, target, &target_id, registry);
    }
    if schema
        .get("const")
        .is_some_and(|constant| constant != value)
    {
        return false;
    }
    if schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|variants| !variants.contains(value))
    {
        return false;
    }
    if schema
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|schemas| {
            schemas
                .iter()
                .any(|nested| !validate_schema(value, nested, owner_id, registry))
        })
    {
        return false;
    }
    if schema
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|schemas| {
            schemas
                .iter()
                .filter(|nested| validate_schema(value, nested, owner_id, registry))
                .count()
                != 1
        })
    {
        return false;
    }
    if schema
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|schemas| {
            !schemas
                .iter()
                .any(|nested| validate_schema(value, nested, owner_id, registry))
        })
    {
        return false;
    }
    if let Some(expected) = schema.get("type") {
        let matches = match expected {
            Value::String(expected) => json_type_matches(value, expected),
            Value::Array(expected) => expected
                .iter()
                .filter_map(Value::as_str)
                .any(|expected| json_type_matches(value, expected)),
            _ => false,
        };
        if !matches {
            return false;
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            return false;
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let Ok(expression) = Regex::new(&format!("^(?:{pattern})$")) else {
                return false;
            };
            if !expression.is_match(text) {
                return false;
            }
        }
    }
    if let Some(values) = value.as_array() {
        let length = values.len() as u64;
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            return false;
        }
        if schema.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
            let unique = values
                .iter()
                .filter_map(|item| serde_json::to_string(item).ok())
                .collect::<BTreeSet<_>>();
            if unique.len() != values.len() {
                return false;
            }
        }
        if let Some(item_schema) = schema.get("items")
            && values
                .iter()
                .any(|item| !validate_schema(item, item_schema, owner_id, registry))
        {
            return false;
        }
    }
    if let Some(object) = value.as_object() {
        if schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|key| !object.contains_key(key))
            })
        {
            return false;
        }
        let length = object.len() as u64;
        if schema
            .get("minProperties")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| length < minimum)
            || schema
                .get("maxProperties")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| length > maximum)
        {
            return false;
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, item) in object {
            if let Some(property_schema) = properties.and_then(|known| known.get(key)) {
                if !validate_schema(item, property_schema, owner_id, registry) {
                    return false;
                }
                continue;
            }
            match schema.get("additionalProperties") {
                Some(Value::Bool(false)) => return false,
                Some(additional @ Value::Object(_))
                    if !validate_schema(item, additional, owner_id, registry) =>
                {
                    return false;
                }
                _ => {}
            }
        }
    }
    if let Some(number) = value.as_f64()
        && (schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|minimum| number < minimum)
            || schema
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|maximum| number > maximum))
    {
        return false;
    }
    true
}

pub(crate) fn json_type_matches(value: &Value, expected: &str) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

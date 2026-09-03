use super::schema::{SchemaRegistry, strict_value};
use super::{ARTIFACTS, ContractError, ContractErrorCode, MAX_ARTIFACT_BYTES};
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) fn schema_registry() -> Result<SchemaRegistry, ContractError> {
    let mut registry = BTreeMap::new();
    for artifact in ARTIFACTS
        .iter()
        .filter(|artifact| artifact.path.ends_with(".schema.json"))
    {
        let schema = strict_value(artifact.bytes).map_err(|_| {
            ContractError::new(ContractErrorCode::SchemaInvalid, Some(artifact.path))
        })?;
        let object = schema.as_object().ok_or_else(|| {
            ContractError::new(ContractErrorCode::SchemaInvalid, Some(artifact.path))
        })?;
        let schema_id = object
            .get("$id")
            .and_then(Value::as_str)
            .filter(|value| value.starts_with("https://abbey.local/contracts/abbey/v1/schemas/"))
            .ok_or_else(|| {
                ContractError::new(ContractErrorCode::SchemaInvalid, Some(artifact.path))
            })?;
        if object.get("$schema").and_then(Value::as_str)
            != Some("https://json-schema.org/draft/2020-12/schema")
            || !object.contains_key("x-abbey-data-class")
            || !object
                .get("x-abbey-max-bytes")
                .and_then(Value::as_u64)
                .is_some_and(|value| value > 0 && value <= MAX_ARTIFACT_BYTES as u64)
            || !matches!(
                object.get("x-abbey-unknown-fields").and_then(Value::as_str),
                Some("reject" | "extensions-only")
            )
            || registry.insert(schema_id.to_owned(), schema).is_some()
        {
            return Err(ContractError::new(
                ContractErrorCode::SchemaInvalid,
                Some(artifact.path),
            ));
        }
    }
    for (schema_id, schema) in &registry {
        if !references_are_local(schema, schema_id, &registry) {
            return Err(ContractError::new(ContractErrorCode::SchemaInvalid, None));
        }
    }
    Ok(registry)
}

pub(crate) fn references_are_local(
    value: &Value,
    owner_id: &str,
    registry: &SchemaRegistry,
) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref") {
                let Some(reference) = reference.as_str() else {
                    return false;
                };
                let base = reference.split('#').next().unwrap_or_default();
                let target = if base.is_empty() { owner_id } else { base };
                if !registry.contains_key(target) {
                    return false;
                }
            }
            object
                .values()
                .all(|nested| references_are_local(nested, owner_id, registry))
        }
        Value::Array(values) => values
            .iter()
            .all(|nested| references_are_local(nested, owner_id, registry)),
        _ => true,
    }
}

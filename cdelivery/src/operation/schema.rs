use serde_json::Value;

pub(super) fn object<T: utoipa::ToSchema>() -> serde_json::Map<String, Value> {
    let mut root = serde_json::json!(<T as utoipa::PartialSchema>::schema());
    normalize(&mut root);
    rewrite_refs(&mut root);
    let mut components = Vec::new();
    T::schemas(&mut components);
    let mut defs = serde_json::Map::new();
    for (name, component) in components {
        let mut value = serde_json::json!(component);
        normalize(&mut value);
        rewrite_refs(&mut value);
        defs.insert(name, value);
    }
    let Value::Object(mut root) = root else {
        return serde_json::Map::from_iter([("type".to_owned(), Value::from("object"))]);
    };
    if !defs.is_empty() {
        root.insert("$defs".to_owned(), Value::Object(defs));
    }
    root
}

pub(crate) fn normalize_openapi(api: &mut utoipa::openapi::OpenApi) -> serde_json::Result<()> {
    let mut value = serde_json::to_value(&*api)?;
    normalize(&mut value);
    *api = serde_json::from_value(value)?;
    Ok(())
}

pub(super) fn output<T: utoipa::ToSchema>() -> serde_json::Map<String, Value> {
    let mut root = object::<T>();
    unrequire_write_only(&mut root);
    root
}

fn unrequire_write_only(schema: &mut serde_json::Map<String, Value>) {
    let write_only: Vec<String> = match schema.get("properties") {
        Some(Value::Object(properties)) => properties
            .iter()
            .filter(|(_, property)| property.get("writeOnly") == Some(&Value::Bool(true)))
            .map(|(name, _)| name.clone())
            .collect(),
        _ => Vec::new(),
    };
    if !write_only.is_empty()
        && let Some(Value::Array(required)) = schema.get_mut("required")
    {
        required.retain(|name| {
            !name
                .as_str()
                .is_some_and(|name| write_only.iter().any(|field| field == name))
        });
        if required.is_empty() {
            schema.remove("required");
        }
    }
    schema.values_mut().for_each(descend);
}

fn descend(value: &mut Value) {
    match value {
        Value::Object(map) => unrequire_write_only(map),
        Value::Array(items) => items.iter_mut().for_each(descend),
        _ => {}
    }
}

fn normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("object")
                && map.get("properties").is_some_and(Value::is_object)
                && !map.contains_key("additionalProperties")
            {
                map.insert("additionalProperties".to_owned(), Value::Bool(false));
            }
            if let Some(Value::String(rule)) = map.remove("x-rule") {
                let description = match map.get("description") {
                    Some(Value::String(doc)) => format!("{doc} {rule}"),
                    _ => rule,
                };
                map.insert("description".to_owned(), description.into());
            }
            map.values_mut().for_each(normalize);
            if !map.contains_key("description")
                && let Some(Value::Array(branches)) = map.get_mut("oneOf")
                && let [Value::Object(branch), null] = branches.as_mut_slice()
                && *null == serde_json::json!({"type": "null"})
                && let Some(description) = branch.remove("description")
            {
                map.insert("description".to_owned(), description);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize),
        _ => {}
    }
}

fn rewrite_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if key == "$ref" {
                    if let Value::String(target) = entry
                        && let Some(name) = target.strip_prefix("#/components/schemas/")
                    {
                        *target = format!("#/$defs/{name}");
                    }
                } else {
                    rewrite_refs(entry);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(rewrite_refs),
        _ => {}
    }
}

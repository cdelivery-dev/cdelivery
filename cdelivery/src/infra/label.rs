use std::collections::HashMap;

pub(crate) fn managed(labels: &mut HashMap<String, String>) {
    labels.insert(insight_wire::MANAGED_LABEL.to_owned(), "true".to_owned());
}

pub(crate) fn json(value: &impl serde::Serialize) -> String {
    serde_json::json!(value).to_string()
}

pub(crate) fn single(key: &str, value: &impl serde::Serialize) -> HashMap<String, String> {
    HashMap::from([(key.to_owned(), json(value))])
}

pub(crate) fn marker(key: &str) -> HashMap<String, String> {
    HashMap::from([(key.to_owned(), String::new())])
}

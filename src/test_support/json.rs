pub fn json_field<'a>(value: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    value.pointer(path).expect("required JSON field")
}

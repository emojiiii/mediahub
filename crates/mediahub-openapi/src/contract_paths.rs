// OpenAPI path operation definitions.

pub fn paths() -> Value {
    let mut paths = Map::new();
    for contract in OPERATIONS {
        let mut operation = Map::new();
        operation.insert("summary".into(), Value::String(contract.summary.into()));
        operation.insert("security".into(), contract.auth.security());
        if !contract.parameters.is_empty() {
            operation.insert(
                "parameters".into(),
                Value::Array(
                    contract
                        .parameters
                        .iter()
                        .map(|name| json!({ "$ref": format!("#/components/parameters/{name}") }))
                        .collect(),
                ),
            );
        }
        if let Some(body) = contract.request {
            operation.insert("requestBody".into(), request_value(body));
        }
        let responses = contract
            .responses
            .iter()
            .map(|response| (response.status.into(), response_value(*response)))
            .collect();
        operation.insert("responses".into(), Value::Object(responses));

        let path_item = paths
            .entry(contract.path)
            .or_insert_with(|| Value::Object(Map::new()));
        path_item
            .as_object_mut()
            .expect("path item is an object")
            .insert(contract.method.as_str().into(), Value::Object(operation));
    }
    Value::Object(paths)
}


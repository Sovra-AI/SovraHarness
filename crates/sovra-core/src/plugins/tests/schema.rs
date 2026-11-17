use super::{CATALOG, PACKAGE};
use crate::plugins::{PluginCatalog, PluginEntry};
use serde_json::{Value, json};

fn validator() -> jsonschema::Validator {
    let schema = serde_json::from_slice(include_bytes!(
        "../../../../../schemas/plugins-v1.schema.json"
    ))
    .unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap()
}

fn package_is_valid(value: &Value) -> bool {
    PluginEntry::for_package(
        "https://example.com/package.json".into(),
        &serde_json::to_vec(value).unwrap(),
    )
    .is_ok()
}

#[test]
fn published_schema_accepts_packages_catalogs_and_optional_contributions() {
    let schema = validator();
    let mut package: Value = serde_json::from_slice(PACKAGE).unwrap();
    let mut catalog: Value = serde_json::from_slice(CATALOG).unwrap();
    schema.validate(&package).unwrap();
    schema.validate(&catalog).unwrap();
    package.as_object_mut().unwrap().remove("skills");
    package["services"] = json!([{"name": "Planner", "address": "https://example.com/mcp"}]);
    schema.validate(&package).unwrap();
    assert!(package_is_valid(&package));
    package["services"] = json!([]);
    assert!(!schema.is_valid(&package));
    assert!(!package_is_valid(&package));

    // Discovery can display an unsupported API without admitting its package.
    catalog["plugins"][0]["api_version"] = json!(2);
    schema.validate(&catalog).unwrap();
    PluginCatalog::parse(&serde_json::to_vec(&catalog).unwrap()).unwrap();
    catalog["plugins"] = json!([]);
    schema.validate(&catalog).unwrap();
    PluginCatalog::parse(&serde_json::to_vec(&catalog).unwrap()).unwrap();
}

#[test]
fn schema_and_installer_reject_contract_shape_changes() {
    let schema = validator();
    let package: Value = serde_json::from_slice(PACKAGE).unwrap();
    for (pointer, replacement) in [
        ("/schema_version", json!(2)),
        ("/api_version", json!(2)),
        ("/version", json!("01.0.0")),
        ("/id", json!("not-a-namespace")),
        ("/title", json!(" ")),
        ("/skills/0/name", json!("../outside")),
        ("/skills/0/instructions", json!(" \n\t")),
        ("/skills", json!(vec![package["skills"][0].clone(); 17])),
        ("/services", json!(null)),
    ] {
        let mut changed = package.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert!(!schema.is_valid(&changed), "schema accepted {pointer}");
        assert!(!package_is_valid(&changed), "installer accepted {pointer}");
    }
    for pointer in ["", "/skills/0"] {
        let mut changed = package.clone();
        changed.pointer_mut(pointer).unwrap()["command"] = json!("run-me");
        assert!(!schema.is_valid(&changed));
        assert!(!package_is_valid(&changed));
    }
    for (pointer, replacement) in [
        ("/plugins/0/sha256", json!("BAD-CHECKSUM")),
        ("/plugins/0/api_version", json!(-1)),
        ("/schema_version", json!(2)),
    ] {
        let mut catalog: Value = serde_json::from_slice(CATALOG).unwrap();
        *catalog.pointer_mut(pointer).unwrap() = replacement;
        assert!(!schema.is_valid(&catalog));
        assert!(PluginCatalog::parse(&serde_json::to_vec(&catalog).unwrap()).is_err());
    }
}

#[test]
fn publisher_validation_covers_constraints_beyond_json_schema() {
    let schema = validator();
    let package: Value = serde_json::from_slice(PACKAGE).unwrap();
    for (pointer, replacement) in [
        ("/title", json!("é".repeat(61))),
        ("/version", json!("4294967296.0.0")),
        ("/skills", json!(vec![package["skills"][0].clone(); 2])),
        (
            "/services",
            json!([{"name": "Planner", "address": "http://example.com/mcp"}]),
        ),
    ] {
        let mut changed = package.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        schema.validate(&changed).unwrap();
        assert!(!package_is_valid(&changed), "installer accepted {pointer}");
    }
    let mut catalog: Value = serde_json::from_slice(CATALOG).unwrap();
    catalog["plugins"] = json!(vec![catalog["plugins"][0].clone(); 2]);
    schema.validate(&catalog).unwrap();
    assert!(PluginCatalog::parse(&serde_json::to_vec(&catalog).unwrap()).is_err());
}

//! A GraphQL API's schema, from its introspection, for data rules that see
//! past field names (ADR 0086): `vehicle { pos { a b } }` carries location
//! when `pos` is a `GeoPoint`, whatever its fields are called. A request is
//! walked with the schema, aliases, fragments and all, and each selected
//! field's type checked.

use std::collections::{BTreeMap, BTreeSet};

use apollo_parser::cst;
use serde_json::Value;

use crate::data::{DataClass, Finding, names_class};

/// The introspection query a plugin runs to learn its API's schema.
pub const INTROSPECTION: &str = "query SiltIntrospection { __schema { queryType { name } mutationType { name } \
subscriptionType { name } types { name kind fields(includeDeprecated: true) { name type { ...Named } } } } } \
fragment Named on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name \
ofType { kind name } } } } } }";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    query: String,
    mutation: Option<String>,
    subscription: Option<String>,
    /// Each type's fields, and the type each field's value has, lists and
    /// non-nulls unwrapped.
    types: BTreeMap<String, BTreeMap<String, String>>,
}

fn named(kind: &Value) -> Option<String> {
    let mut at = kind;
    for _ in 0..8 {
        if let Some(name) = at.get("name").and_then(Value::as_str) {
            return Some(name.to_owned());
        }
        at = at.get("ofType")?;
    }
    None
}

impl Schema {
    /// From an introspection's answer, with or without its `data`.
    pub fn from_introspection(answer: &Value) -> Result<Self, String> {
        let schema = answer.pointer("/data/__schema").or_else(|| answer.get("__schema")).ok_or("no __schema in the answer")?;
        let root = |name: &str| schema.pointer(&format!("/{name}/name")).and_then(Value::as_str).map(str::to_owned);
        let mut types = BTreeMap::new();
        for kind in schema.get("types").and_then(Value::as_array).ok_or("no types in the schema")? {
            let Some(name) = kind.get("name").and_then(Value::as_str) else { continue };
            let fields = kind
                .get("fields")
                .and_then(Value::as_array)
                .map(|fields| {
                    fields
                        .iter()
                        .filter_map(|field| Some((field.get("name")?.as_str()?.to_owned(), named(field.get("type")?)?)))
                        .collect()
                })
                .unwrap_or_default();
            types.insert(name.to_owned(), fields);
        }
        Ok(Self {
            query: root("queryType").ok_or("the schema has no query type")?,
            mutation: root("mutationType"),
            subscription: root("subscriptionType"),
            types,
        })
    }

    /// Whether values of type `name` carry `class`: its name says so, or
    /// it's made of fields that do, such as a point's `lat` and `lng`.
    pub fn carries(&self, class: DataClass, name: &str) -> bool {
        if names_class(class, name) {
            return true;
        }
        let Some(fields) = self.types.get(name) else { return false };
        fields.keys().filter(|field| names_class(class, field)).count() >= 2
    }

    /// The first field `document` selects whose value carries `class`, by
    /// its name or its type's.
    pub fn finding(&self, class: DataClass, document: &str) -> Result<Option<Finding>, String> {
        let tree = apollo_parser::Parser::new(document).parse();
        if let Some(error) = tree.errors().next() {
            return Err(format!("the request isn't valid GraphQL: {}", error.message()));
        }
        let mut fragments = BTreeMap::new();
        let mut operations = Vec::new();
        for definition in tree.document().definitions() {
            match definition {
                cst::Definition::FragmentDefinition(fragment) => {
                    let name = fragment.fragment_name().and_then(|name| name.name()).map(|name| name.text().to_string());
                    let on = fragment
                        .type_condition()
                        .and_then(|condition| condition.named_type())
                        .and_then(|named| named.name())
                        .map(|name| name.text().to_string());
                    if let (Some(name), Some(on), Some(selections)) = (name, on, fragment.selection_set()) {
                        fragments.insert(name, (on, selections));
                    }
                }
                cst::Definition::OperationDefinition(operation) => {
                    let root = match operation.operation_type() {
                        Some(kind) if kind.mutation_token().is_some() => self.mutation.clone(),
                        Some(kind) if kind.subscription_token().is_some() => self.subscription.clone(),
                        _ => Some(self.query.clone()),
                    }
                    .ok_or("the schema has no root type for an operation of the request")?;
                    if let Some(selections) = operation.selection_set() {
                        operations.push((root, selections));
                    }
                }
                _ => return Err("a request holds operations and fragments only".into()),
            }
        }
        let walk = Walk { schema: self, class, fragments: &fragments };
        for (root, selections) in operations {
            if let Some(found) = walk.selections(&root, &selections, &mut BTreeSet::new(), "")? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

struct Walk<'a> {
    schema: &'a Schema,
    class: DataClass,
    fragments: &'a BTreeMap<String, (String, cst::SelectionSet)>,
}

impl Walk<'_> {
    fn selections(
        &self,
        parent: &str,
        selections: &cst::SelectionSet,
        spread: &mut BTreeSet<String>,
        path: &str,
    ) -> Result<Option<Finding>, String> {
        for selection in selections.selections() {
            let found = match selection {
                cst::Selection::Field(field) => {
                    let Some(name) = field.name().map(|name| name.text().to_string()) else { continue };
                    if name.starts_with("__") {
                        continue;
                    }
                    let kind = self
                        .schema
                        .types
                        .get(parent)
                        .and_then(|fields| fields.get(&name))
                        .ok_or_else(|| format!("{parent} has no field {name} in the schema Silt knows"))?;
                    let here = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
                    if names_class(self.class, &name) || self.schema.carries(self.class, kind) {
                        return Ok(Some(Finding {
                            class: self.class,
                            what: format!("it selects {here}, a {kind}, which carries {}", self.class.label()),
                        }));
                    }
                    match field.selection_set() {
                        Some(inner) => self.selections(kind, &inner, spread, &here)?,
                        None => None,
                    }
                }
                cst::Selection::InlineFragment(fragment) => {
                    let on = fragment
                        .type_condition()
                        .and_then(|condition| condition.named_type())
                        .and_then(|named| named.name())
                        .map_or_else(|| parent.to_owned(), |name| name.text().to_string());
                    match fragment.selection_set() {
                        Some(inner) => self.selections(&on, &inner, spread, path)?,
                        None => None,
                    }
                }
                cst::Selection::FragmentSpread(fragment) => {
                    let Some(name) = fragment.fragment_name().and_then(|name| name.name()).map(|name| name.text().to_string())
                    else {
                        continue;
                    };
                    // A fragment spread twice on one path is walked once.
                    if !spread.insert(name.clone()) {
                        continue;
                    }
                    let (on, inner) = self.fragments.get(&name).ok_or_else(|| format!("the request has no fragment {name}"))?;
                    let found = self.selections(on, inner, spread, path)?;
                    spread.remove(&name);
                    found
                }
            };
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Schema {
        let field = |name: &str, kind: &str| json!({"name": name, "type": {"kind": "NON_NULL", "name": null, "ofType": {"kind": "OBJECT", "name": kind}}});
        let answer = json!({"data": {"__schema": {
            "queryType": {"name": "Query"},
            "mutationType": {"name": "Mutation"},
            "subscriptionType": null,
            "types": [
                {"name": "Query", "kind": "OBJECT", "fields": [field("vehicle", "Vehicle"), field("vehicles", "Vehicle")]},
                {"name": "Mutation", "kind": "OBJECT", "fields": [field("setOta", "Vehicle")]},
                {"name": "Vehicle", "kind": "OBJECT", "fields": [field("id", "ID"), field("name", "String"), field("pos", "Fix"), field("trips", "Trip")]},
                {"name": "Trip", "kind": "OBJECT", "fields": [field("id", "ID"), field("end", "Spot")]},
                {"name": "Fix", "kind": "OBJECT", "fields": [field("lat", "Float"), field("lng", "Float")]},
                {"name": "Spot", "kind": "SCALAR", "fields": null},
                {"name": "GeoJSON", "kind": "SCALAR", "fields": null},
                {"name": "ID", "kind": "SCALAR", "fields": null},
                {"name": "String", "kind": "SCALAR", "fields": null},
                {"name": "Float", "kind": "SCALAR", "fields": null}
            ]
        }}});
        Schema::from_introspection(&answer).unwrap()
    }

    #[test]
    fn a_field_whose_type_carries_location_is_found_whatever_its_name() {
        let schema = schema();
        let location = DataClass::Location;
        assert!(schema.finding(location, "{ vehicle { id name } }").unwrap().is_none());
        let found = schema.finding(location, "{ vehicle { id where: pos { a: lat } } }").unwrap().unwrap();
        assert!(found.what.contains("vehicle.pos, a Fix"), "{}", found.what);
        assert!(schema.carries(location, "GeoJSON"), "by its name");
        assert!(!schema.carries(location, "Spot"), "a scalar whose name says nothing");
    }

    #[test]
    fn fragments_and_mutations_are_walked_too() {
        let schema = schema();
        let found = schema
            .finding(
                DataClass::Location,
                "query { vehicles { ...Car } } fragment Car on Vehicle { ... on Vehicle { pos { lat } } }",
            )
            .unwrap();
        assert!(found.is_some());
        assert!(schema.finding(DataClass::Location, "mutation { setOta { id pos { lat } } }").unwrap().is_some());
        let looping = "query { vehicle { ...A } } fragment A on Vehicle { id ...A }";
        assert!(schema.finding(DataClass::Location, looping).unwrap().is_none(), "a fragment spread in itself ends");
    }

    #[test]
    fn what_the_schema_doesnt_know_is_refused() {
        let error = schema().finding(DataClass::Location, "{ vehicle { secretField } }").unwrap_err();
        assert!(error.contains("no field secretField"), "{error}");
    }
}

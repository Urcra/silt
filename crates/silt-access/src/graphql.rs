//! A GraphQL document read for what it does: whether any of its operations
//! changes something, and every field it selects, wherever, so a rule can
//! refuse it before it's sent.

use std::collections::BTreeSet;

use apollo_parser::cst::{self, CstNode as _};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub operations: Vec<OperationKind>,
    /// Every field the document selects, in its operations and fragments,
    /// by name rather than alias.
    pub fields: BTreeSet<String>,
}

impl Classified {
    /// Whether it may change something: any mutation or subscription in
    /// it counts, whichever operation the request names.
    pub fn changes(&self) -> bool {
        self.operations.iter().any(|kind| *kind != OperationKind::Query)
    }
}

/// Reads `document`, refusing one that doesn't parse or holds anything but
/// operations and fragments.
pub fn classify(document: &str) -> Result<Classified, String> {
    let tree = apollo_parser::Parser::new(document).parse();
    if let Some(error) = tree.errors().next() {
        return Err(format!("the request isn't valid GraphQL: {}", error.message()));
    }
    let mut operations = Vec::new();
    let mut fields = BTreeSet::new();
    for definition in tree.document().definitions() {
        match definition {
            cst::Definition::OperationDefinition(operation) => {
                let kind = match operation.operation_type() {
                    Some(kind) if kind.mutation_token().is_some() => OperationKind::Mutation,
                    Some(kind) if kind.subscription_token().is_some() => OperationKind::Subscription,
                    _ => OperationKind::Query,
                };
                operations.push(kind);
                if let Some(selections) = operation.selection_set() {
                    collect(&selections, &mut fields);
                }
            }
            cst::Definition::FragmentDefinition(fragment) => {
                if let Some(selections) = fragment.selection_set() {
                    collect(&selections, &mut fields);
                }
            }
            other => return Err(format!("a request holds operations and fragments only, not {:?}", other.syntax().kind())),
        }
    }
    if operations.is_empty() {
        return Err("the request holds no operation".into());
    }
    Ok(Classified { operations, fields })
}

fn collect(selections: &cst::SelectionSet, fields: &mut BTreeSet<String>) {
    for selection in selections.selections() {
        match selection {
            cst::Selection::Field(field) => {
                if let Some(name) = field.name() {
                    fields.insert(name.text().to_string());
                }
                if let Some(inner) = field.selection_set() {
                    collect(&inner, fields);
                }
            }
            cst::Selection::InlineFragment(fragment) => {
                if let Some(inner) = fragment.selection_set() {
                    collect(&inner, fields);
                }
            }
            cst::Selection::FragmentSpread(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mutation_anywhere_is_a_change() {
        let read = classify("{ vehicle(id: 1) { id name } }").unwrap();
        assert!(!read.changes());
        assert_eq!(read.fields, BTreeSet::from(["vehicle".into(), "id".into(), "name".into()]));
        let two = classify("query A { a } mutation B { setOta(on: true) { id } }").unwrap();
        assert!(two.changes(), "whatever operationName picks");
        assert!(classify("subscription { trips { id } }").unwrap().changes());
    }

    #[test]
    fn aliases_and_fragments_hide_no_field() {
        let sneaky = classify(
            "query { car { where: position { lat lon } ...Spot } } fragment Spot on Car { ... on Car { geo { latitude } } }",
        )
        .unwrap();
        for field in ["position", "lat", "lon", "geo", "latitude"] {
            assert!(sneaky.fields.contains(field), "{field}: {:?}", sneaky.fields);
        }
        assert!(!sneaky.fields.contains("where"), "an alias isn't a field");
    }

    #[test]
    fn what_isnt_a_request_is_refused() {
        assert!(classify("{ unclosed").is_err());
        assert!(classify("type Car { id: ID }").unwrap_err().contains("operations and fragments only"));
        assert!(classify("").is_err());
    }
}

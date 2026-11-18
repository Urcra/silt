//! Which of a VS Code theme's `tokenColors` rules styles a TextMate scope,
//! as VS Code picks it: the most specific selector, the later rule on a tie.

use serde_json::Value;

/// One rule's selector and what it sets.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: Vec<String>,
    pub foreground: Option<String>,
    pub font_style: Option<String>,
}

/// The rules of a `tokenColors` list; those without a scope (the editor's
/// defaults) are left out.
pub fn rules(token_colors: &Value) -> Vec<Rule> {
    let Some(list) = token_colors.as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|entry| {
            let selectors: Vec<String> = match entry.get("scope")? {
                Value::String(text) => text.split(',').map(|part| part.trim().to_owned()).collect(),
                Value::Array(items) => items.iter().filter_map(Value::as_str).map(|part| part.trim().to_owned()).collect(),
                _ => return None,
            };
            let settings = entry.get("settings")?;
            Some(Rule {
                selectors: selectors.into_iter().filter(|selector| !selector.is_empty()).collect(),
                foreground: settings.get("foreground").and_then(Value::as_str).map(str::to_owned),
                font_style: settings.get("fontStyle").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .collect()
}

/// How well `selector` matches `scope`, if at all: the number of dotted parts
/// it pins, less for a selector that also asks about ancestors, which Silt
/// can't see.
fn score(selector: &str, scope: &str) -> Option<f64> {
    let selector = selector.split(" -").next().unwrap_or(selector).trim();
    let mut parts = selector.split_whitespace();
    let last = parts.next_back()?;
    let has_ancestors = parts.next().is_some();
    let matches = scope == last || scope.strip_prefix(last).is_some_and(|rest| rest.starts_with('.'));
    matches.then(|| last.split('.').count() as f64 - if has_ancestors { 0.5 } else { 0.0 })
}

/// The best rule for `scope` that sets what `wants` asks for.
pub fn best<'a>(rules: &'a [Rule], scope: &str, wants: impl Fn(&Rule) -> bool) -> Option<&'a Rule> {
    let mut found: Option<(f64, &Rule)> = None;
    for rule in rules.iter().filter(|rule| wants(rule)) {
        let Some(rule_score) = rule.selectors.iter().filter_map(|selector| score(selector, scope)).reduce(f64::max) else {
            continue;
        };
        if found.is_none_or(|(best, _)| rule_score >= best) {
            found = Some((rule_score, rule));
        }
    }
    found.map(|(_, rule)| rule)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_most_specific_rule_wins_and_a_tie_goes_to_the_later() {
        let rules = rules(&json!([
            { "settings": { "foreground": "#ffffff" } },
            { "scope": "keyword", "settings": { "foreground": "#111111" } },
            { "scope": ["keyword.control", "storage"], "settings": { "foreground": "#222222" } },
            { "scope": "keyword.control", "settings": { "foreground": "#333333" } },
            { "scope": "source.rust keyword.control.flow", "settings": { "foreground": "#444444" } },
            { "scope": "keyword.operator, punctuation", "settings": { "fontStyle": "italic" } },
        ]));
        assert_eq!(rules.len(), 5, "the scopeless default is left out");
        let color = |scope| best(&rules, scope, |rule| rule.foreground.is_some()).and_then(|rule| rule.foreground.clone());
        assert_eq!(color("keyword.control.rust").as_deref(), Some("#333333"));
        assert_eq!(color("keyword.control.flow.rust").as_deref(), Some("#444444"), "3 parts with ancestors beat 2");
        assert_eq!(color("keyword.other").as_deref(), Some("#111111"));
        assert_eq!(color("keywords"), None, "a prefix only counts at a dot");
        let style =
            best(&rules, "punctuation.separator", |rule| rule.font_style.is_some()).and_then(|rule| rule.font_style.clone());
        assert_eq!(style.as_deref(), Some("italic"));
    }
}

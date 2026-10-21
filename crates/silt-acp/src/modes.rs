//! What an agent lets the person choose for its session: its modes, and
//! settings such as its model (ACP's session config options), simplified.

use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigSelectOption, SessionConfigSelectOptions, SessionModeState,
};

/// A mode, or one value of a setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modes {
    pub current: String,
    pub available: Vec<Choice>,
}

/// A setting with one value of several, such as the model; settings of
/// other kinds are left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub id: String,
    pub name: String,
    /// `model`, `thought_level`, … as the agent tags it.
    pub category: Option<String>,
    pub current: String,
    pub choices: Vec<Choice>,
}

pub(crate) fn modes_of(state: Option<SessionModeState>) -> Option<Modes> {
    let state = state?;
    Some(Modes {
        current: state.current_mode_id.0.to_string(),
        available: state
            .available_modes
            .into_iter()
            .map(|mode| Choice { id: mode.id.0.to_string(), name: mode.name, description: mode.description })
            .collect(),
    })
}

pub(crate) fn settings_of(options: Vec<SessionConfigOption>) -> Vec<Setting> {
    options.into_iter().filter_map(setting_of).collect()
}

fn setting_of(option: SessionConfigOption) -> Option<Setting> {
    let SessionConfigKind::Select(select) = option.kind else {
        return None;
    };
    let values: Vec<SessionConfigSelectOption> = match select.options {
        SessionConfigSelectOptions::Ungrouped(values) => values,
        SessionConfigSelectOptions::Grouped(groups) => groups.into_iter().flat_map(|group| group.options).collect(),
        _ => return None,
    };
    Some(Setting {
        id: option.id.0.to_string(),
        name: option.name,
        category: option.category.as_ref().map(crate::host::label),
        current: select.current_value.0.to_string(),
        choices: values
            .into_iter()
            .map(|value| Choice { id: value.value.0.to_string(), name: value.name, description: value.description })
            .collect(),
    })
}

/// The mode to plan in, when the agent has one.
pub fn plan_mode(modes: &Modes) -> Option<&Choice> {
    modes.available.iter().find(|mode| mode.id.to_lowercase().contains("plan") || mode.name.to_lowercase().contains("plan"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_mode_is_found_by_id_or_name() {
        let choice = |id: &str, name: &str| Choice { id: id.into(), name: name.into(), description: None };
        let modes = Modes { current: "default".into(), available: vec![choice("default", "Default"), choice("p", "Plan Mode")] };
        assert_eq!(plan_mode(&modes).map(|mode| mode.id.as_str()), Some("p"));
        let none = Modes { current: "a".into(), available: vec![choice("a", "Ask"), choice("code", "Code")] };
        assert!(plan_mode(&none).is_none());
    }
}

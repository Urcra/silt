//! What an agent lets the person choose for its session: its modes,
//! settings such as its model (ACP's session config options), and the
//! commands a prompt can run, simplified.

use agent_client_protocol::schema::v1::{
    self, SessionConfigKind, SessionConfigOption, SessionConfigSelectOption, SessionConfigSelectOptions, SessionModeState,
};
use agent_client_protocol::schema::v2;

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

/// A command the agent offers, run by a prompt that starts with `/` and its
/// name (ACP's available commands).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    /// What to write after it, such as `<PR URL>`, when it takes anything.
    pub hint: Option<String>,
}

pub(crate) fn commands_of(commands: Vec<v1::AvailableCommand>) -> Vec<SlashCommand> {
    commands
        .into_iter()
        .map(|command| SlashCommand {
            hint: match command.input {
                Some(v1::AvailableCommandInput::Unstructured(input)) => Some(input.hint),
                _ => None,
            },
            name: command.name,
            description: command.description,
        })
        .collect()
}

pub(crate) fn commands_v2(commands: Vec<v2::AvailableCommand>) -> Vec<SlashCommand> {
    commands
        .into_iter()
        .map(|command| SlashCommand {
            hint: match command.input {
                Some(v2::AvailableCommandInput::Text(input)) => Some(input.hint),
                _ => None,
            },
            name: command.name,
            description: command.description,
        })
        .collect()
}

/// Whether the agent, by its registry id, puts its plan to the person
/// itself, asking leave to quit its plan mode as Claude's ExitPlanMode does,
/// so a turn it ends in plan mode isn't a plan to approve: it may be waiting
/// on its subagents, or asking something.
pub fn presents_plans(agent: &str) -> bool {
    agent == "claude-acp"
}

/// What an agent is known to offer before it has run on this machine, by
/// its registry id: what a new thread shows until the agent itself says,
/// as Claude's adapter does when its session opens. A value it no longer
/// offers is dropped when it starts.
pub fn built_in(agent: &str) -> Option<(Modes, Vec<Setting>)> {
    let choice = |id: &str, name: &str| Choice { id: id.to_owned(), name: name.to_owned(), description: None };
    match agent {
        "claude-acp" => Some((
            Modes {
                current: "default".to_owned(),
                available: vec![
                    choice("default", "Manual"),
                    choice("acceptEdits", "Accept edits"),
                    choice("plan", "Plan"),
                    choice("auto", "Auto"),
                ],
            },
            vec![
                Setting {
                    id: "model".to_owned(),
                    name: "Model".to_owned(),
                    category: Some("model".to_owned()),
                    current: "default".to_owned(),
                    choices: vec![
                        choice("default", "Default"),
                        choice("opus", "Opus"),
                        choice("sonnet", "Sonnet"),
                        choice("haiku", "Haiku"),
                    ],
                },
                Setting {
                    id: "effort".to_owned(),
                    name: "Effort".to_owned(),
                    category: Some("thought_level".to_owned()),
                    current: "default".to_owned(),
                    choices: vec![
                        choice("default", "Default"),
                        choice("low", "Low"),
                        choice("medium", "Medium"),
                        choice("high", "High"),
                        choice("xhigh", "XHigh"),
                        choice("max", "Max"),
                    ],
                },
            ],
        )),
        _ => None,
    }
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

/// v2's settings, as v1's: settings of other kinds than choices are left out.
pub(crate) fn settings_v2(options: Vec<v2::SessionConfigOption>) -> Vec<Setting> {
    options.into_iter().filter_map(setting_v2).collect()
}

fn setting_v2(option: v2::SessionConfigOption) -> Option<Setting> {
    let v2::SessionConfigKind::Select(select) = option.kind else {
        return None;
    };
    let values: Vec<v2::SessionConfigSelectOption> = match select.options {
        v2::SessionConfigSelectOptions::Ungrouped(values) => values,
        v2::SessionConfigSelectOptions::Grouped(groups) => groups.into_iter().flat_map(|group| group.options).collect(),
        _ => return None,
    };
    Some(Setting {
        id: option.config_id.0.to_string(),
        name: option.name,
        category: option.category.as_ref().map(crate::host::label),
        current: select.current_value.0.to_string(),
        choices: values
            .into_iter()
            .map(|value| Choice { id: value.value.0.to_string(), name: value.name, description: value.description })
            .collect(),
    })
}

/// A v2 agent's modes: its setting of the `mode` category, which v1 sent
/// apart.
pub(crate) fn modes_in(settings: &[Setting]) -> Option<(String, Modes)> {
    let mode = settings.iter().find(|setting| setting.category.as_deref() == Some("mode"))?;
    Some((mode.id.clone(), Modes { current: mode.current.clone(), available: mode.choices.clone() }))
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

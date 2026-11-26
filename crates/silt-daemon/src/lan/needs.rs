//! What a phone on the network needs to be let make each request (ADR
//! 0112): the capabilities ADR 0028 gives phones, and requests no phone
//! makes, which change this machine itself. Every request is named, so a new
//! one has to be placed.

use silt_proto::Capabilities;

use crate::protocol::Request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Need {
    /// Reading the person's work, and their own marks on it.
    Read,
    AnswerPermissions,
    SendPrompts,
    RunAgents,
    Review,
    Sample,
    PageTools,
    /// Changing this machine: its settings, projects, plugins, models and
    /// what it shares. Only the machine's own UI and CLI.
    Machine,
}

impl Need {
    /// Whether a phone with `capabilities` may; if not, what it would need,
    /// for saying so.
    pub(crate) fn allowed(self, capabilities: &Capabilities) -> Result<(), &'static str> {
        let (allowed, needs) = match self {
            Self::Read => (true, ""),
            Self::AnswerPermissions => (capabilities.answer_permissions, "answer permissions"),
            Self::SendPrompts => (capabilities.send_prompts, "send prompts"),
            Self::RunAgents => (capabilities.run_agents, "start and stop agents"),
            Self::Review => (capabilities.review, "review merge requests"),
            Self::Sample => (capabilities.sample, "ask an agent a page's question"),
            Self::PageTools => (capabilities.page_tools, "call your page connectors"),
            Self::Machine => (false, "change this machine's settings"),
        };
        if allowed { Ok(()) } else { Err(needs) }
    }
}

impl Request {
    pub(crate) fn needs(&self) -> Need {
        use Need::{AnswerPermissions, Machine, PageTools, Read, Review, RunAgents, Sample, SendPrompts};
        match self {
            Self::Ping { .. }
            | Self::Threads { .. }
            | Self::Show { .. }
            | Self::Diff { .. }
            | Self::DiffStats { .. }
            | Self::ThreadSeen { .. }
            | Self::MarkThreadSeen { .. }
            | Self::DiffTurns { .. }
            | Self::Search { .. }
            | Self::Status { .. }
            | Self::MrList { .. }
            | Self::HistoryRead { .. }
            | Self::HistorySearch { .. }
            | Self::Turns { .. }
            | Self::Unwritten { .. }
            | Self::Commits { .. }
            | Self::Graph { .. }
            | Self::FindCommit { .. }
            | Self::FileHistory { .. }
            | Self::InFlight { .. }
            | Self::Agents { .. }
            | Self::Askable { .. }
            | Self::Asks
            | Self::Blame { .. }
            | Self::Comments { .. }
            | Self::AnchorLines { .. }
            | Self::PlaceAnchors { .. }
            | Self::Themes { .. }
            | Self::ThemeDetail { .. }
            | Self::ThemeFont { .. }
            | Self::ThemeCss { .. }
            | Self::ArtifactSettings { .. }
            | Self::DesignSystems { .. }
            | Self::DesignCatalog { .. }
            | Self::DesignSystem { .. }
            | Self::DesignAsset { .. }
            | Self::ThreadDesignSystem { .. }
            | Self::ProjectDesignSystems { .. }
            | Self::ArtifactRead { .. }
            | Self::ArtifactContent { .. }
            | Self::BlobSource { .. }
            | Self::ArtifactVersion { .. }
            | Self::ArtifactFile { .. }
            | Self::ArtifactListing { .. }
            | Self::ArtifactThumbnail { .. }
            | Self::PageViewer { .. }
            | Self::ArtifactGrants { .. }
            | Self::PageAsset { .. }
            | Self::PageData { .. }
            | Self::DocData { .. }
            | Self::SheetData { .. }
            | Self::ArtifactPdf { .. }
            | Self::KeepThumbnail { .. }
            | Self::PinArtifact { .. }
            | Self::ArchiveArtifact { .. }
            | Self::RoomRoute { .. }
            | Self::RoomJoin { .. }
            | Self::RoomSend { .. }
            | Self::RoomLeave { .. }
            | Self::Access { .. }
            | Self::Approvals { .. }
            | Self::Inbox { .. }
            | Self::InboxRead { .. }
            | Self::Trash { .. }
            | Self::Presence { .. }
            | Self::Replica { .. }
            | Self::GitText { .. }
            | Self::Live { .. }
            | Self::AcpAgents { .. }
            | Self::AgentUsage { .. }
            | Self::Branches { .. }
            | Self::CloudLaunches { .. }
            | Self::Projects { .. }
            | Self::ThreadState { .. }
            | Self::Orgs { .. }
            | Self::NotificationSettings { .. }
            | Self::NotificationSetting { .. }
            | Self::Devices { .. }
            | Self::ThreadLinks { .. }
            | Self::Workspaces { .. }
            | Self::Queue { .. }
            | Self::Models { .. }
            | Self::ChatSettings { .. }
            | Self::Update { .. }
            | Self::Plugins { .. }
            | Self::PluginActivity { .. }
            | Self::LanPush { .. } => Read,
            Self::AcpAnswer { .. } | Self::AcpReply { .. } | Self::ApprovalAnswer { .. } => AnswerPermissions,
            Self::AcpPrompt { .. }
            | Self::MayUpload
            | Self::AcpSteer { .. }
            | Self::AcpCancel { .. }
            | Self::QueueAdd { .. }
            | Self::QueueEdit { .. }
            | Self::QueueMove { .. }
            | Self::QueueRemove { .. }
            | Self::QueueSend { .. }
            | Self::CommentsSent { .. }
            | Self::MrGuide { .. }
            | Self::PublishArtifact { .. }
            | Self::SaveEditedVersion { .. } => SendPrompts,
            Self::AcpStart { .. }
            | Self::AcpStop { .. }
            | Self::AcpStopTask { .. }
            | Self::AcpMode { .. }
            | Self::AcpSetting { .. }
            | Self::AgentPreset { .. }
            | Self::AcpSignIn { .. }
            | Self::AcpSignInCode { .. }
            | Self::AcpSignInCancel { .. }
            | Self::Resume { .. }
            | Self::CloudStart { .. }
            | Self::CloudSend { .. } => RunAgents,
            Self::MrShow { .. }
            | Self::MrSeen { .. }
            | Self::FileAt { .. }
            | Self::PageFiles { .. }
            | Self::MrApprove { .. }
            | Self::MrDecline { .. }
            | Self::MrRequestChanges { .. }
            | Self::Comment { .. }
            | Self::Reply { .. }
            | Self::ResolveComment { .. } => Review,
            Self::Sample { .. }
            | Self::SampleToolCall { .. }
            | Self::SampleToolResult { .. }
            | Self::StopSample { .. }
            | Self::RoomAgent { .. }
            | Self::SetArtifactGrant { .. } => Sample,
            Self::PageMcp { .. } | Self::ConnectorTools { .. } => PageTools,
            Self::AgentHomes { .. }
            | Self::DaemonBuild { .. }
            | Self::RestartWhenIdle { .. }
            | Self::StartSession { .. }
            | Self::Hook { .. }
            | Self::EndSession { .. }
            | Self::CloneOf { .. }
            | Self::ThreadErase { .. }
            | Self::ModelPull { .. }
            | Self::ModelRemove { .. }
            | Self::ModelSearch { .. }
            | Self::ModelFiles { .. }
            | Self::RuntimePull { .. }
            | Self::UpdateCheck { .. }
            | Self::UpdateStart { .. }
            | Self::UpdateWaiting { .. }
            | Self::SetChatSettings { .. }
            | Self::MakeProject { .. }
            | Self::Adopt { .. }
            | Self::Reload { .. }
            | Self::ImportCandidates { .. }
            | Self::Import { .. }
            | Self::FoundFolders { .. }
            | Self::ImportFound { .. }
            | Self::Send { .. }
            | Self::Fork { .. }
            | Self::Pull { .. }
            | Self::Revert { .. }
            | Self::MrCreate { .. }
            | Self::OpenWeb { .. }
            | Self::Message { .. }
            | Self::Ask { .. }
            | Self::AskWait { .. }
            | Self::AskStop { .. }
            | Self::AskTake { .. }
            | Self::SetVisibility { .. }
            | Self::OnServer { .. }
            | Self::PutArtifact { .. }
            | Self::ArtifactGuide { .. }
            | Self::SetAppearance { .. }
            | Self::ImportTheme { .. }
            | Self::DuplicateTheme { .. }
            | Self::SetArtifactSettings { .. }
            | Self::PageAssets { .. }
            | Self::PageConnectors { .. }
            | Self::SetPageConnectors { .. }
            | Self::ImportPageConnectors { .. }
            | Self::SetConnectorSecret { .. }
            | Self::DesignStart { .. }
            | Self::DesignSkillUsed { .. }
            | Self::KeepDesignSystem { .. }
            | Self::SetProjectDesignSystem { .. }
            | Self::PluginTools { .. }
            | Self::PluginCall { .. }
            | Self::PluginWait { .. }
            | Self::PluginTurn { .. }
            | Self::PluginRemove { .. }
            | Self::PluginRule { .. }
            | Self::PluginSet { .. }
            | Self::PluginEnvironment { .. }
            | Self::PluginTrust { .. }
            | Self::BindRunToken { .. }
            | Self::RelayDevices { .. }
            | Self::ReleaseDevice { .. }
            | Self::GrantRevoke { .. }
            | Self::FolderRevoke { .. }
            | Self::Scoped { .. }
            | Self::ThreadDelete { .. }
            | Self::ThreadRestore { .. }
            | Self::ThreadPurge { .. }
            | Self::BrowserDeltas { .. }
            | Self::ThreadShare { .. }
            | Self::CloudLaunched { .. }
            | Self::PairPhone { .. }
            | Self::ThreadLink { .. }
            | Self::ThreadLinkRevoke { .. }
            | Self::DeviceChange { .. }
            | Self::DeviceRevoke { .. }
            | Self::OrgChange { .. }
            | Self::AddClone { .. }
            | Self::CloneProject { .. }
            | Self::NewProject { .. }
            | Self::Subfolders { .. }
            | Self::RunRules { .. }
            | Self::SetWorkspaces { .. }
            | Self::AssignProject { .. }
            | Self::ProjectHome { .. }
            | Self::Compact { .. }
            | Self::Shutdown { .. }
            | Self::LanStatus { .. }
            | Self::LanTurn { .. }
            | Self::LanPair { .. }
            | Self::LanAnswer { .. }
            | Self::LanPhoneChange { .. }
            | Self::LanPhoneRevoke { .. }
            | Self::LanForget { .. } => Machine,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread() -> String {
        "535c3cecd5e3".to_owned()
    }

    #[test]
    fn a_phone_reads_and_answers_but_changes_nothing_else_unless_given_more() {
        let phone = Capabilities::PHONE;
        assert_eq!(Request::Threads.needs().allowed(&phone), Ok(()));
        assert_eq!(Request::Status.needs().allowed(&phone), Ok(()));
        assert_eq!(Request::AcpCancel { thread: thread() }.needs().allowed(&phone), Err("send prompts"));
        assert_eq!(Request::AcpCancel { thread: thread() }.needs().allowed(&Capabilities::ALL), Ok(()));
        assert_eq!(Request::LanTurn { on: false }.needs().allowed(&Capabilities::ALL), Err("change this machine's settings"));
        assert_eq!(Request::Shutdown.needs().allowed(&Capabilities::ALL), Err("change this machine's settings"));
    }
}

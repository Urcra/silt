//! Spawns an ACP agent, sends one prompt in a fresh session and prints every
//! `session/update` it streams back.
//!
//! ```sh
//! cargo run -p silt-acp --example acp_smoke -- <workdir> "<prompt>" [agent command]
//! ```

use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, Client};

const DEFAULT_AGENT: &str = "npx -y @agentclientprotocol/claude-agent-acp@0.94.0";

#[tokio::main]
async fn main() -> agent_client_protocol::Result<()> {
    let mut args = env::args().skip(1);
    let workdir = PathBuf::from(args.next().expect("usage: acp_smoke <workdir> <prompt> [agent]"));
    let prompt = args.next().expect("usage: acp_smoke <workdir> <prompt> [agent]");
    let command = args.next().unwrap_or_else(|| DEFAULT_AGENT.to_owned());

    eprintln!("spawning {command}");
    let agent = AcpAgent::from_str(&command)?;

    Client
        .v1()
        .name("silt-acp-smoke")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                println!("{:?}", notification.update);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                eprintln!("auto-approving {:?}", request.tool_call);
                let outcome = match request.options.first() {
                    Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id.clone())),
                    None => RequestPermissionOutcome::Cancelled,
                };
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async move |connection: ConnectionTo<Agent>| {
            let init = connection.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
            eprintln!("agent: {:?}", init.agent_info);
            eprintln!("capabilities: {:?}", init.agent_capabilities);

            let session = connection.send_request(NewSessionRequest::new(workdir)).block_task().await?;

            let response = connection
                .send_request(PromptRequest::new(session.session_id, vec![ContentBlock::Text(TextContent::new(prompt))]))
                .block_task()
                .await?;
            eprintln!("stop reason: {:?}", response.stop_reason);
            Ok(())
        })
        .await
}

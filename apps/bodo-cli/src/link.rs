//! `bodo-cli mcp --connect`: the MCP tools act on the running app through its control channel (project.md sections
//! 12.3 and 12.4), so the user watches the agent work and the agent's edits join the app's undo history.

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};
use bodo_control::{Client, ControlError, status};
use bodo_mcp::Server;
use bodo_mcp::connect::{Failure, Remote};
use serde_json::Value;

/// Which app to talk to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The app that published itself in the user's data folder.
    Running,
    /// The app listening on this port of 127.0.0.1, with the token it published.
    Port(u16),
    /// An app at this address with a known token, for hosts that start their own control server.
    Address {
        /// Where it listens.
        addr: SocketAddr,
        /// Its access token.
        token: String,
    },
}

/// A connection to the app that reconnects on the next request after the app restarted.
#[derive(Debug)]
pub struct Link {
    target: Target,
    client: Option<Client>,
}

impl Link {
    /// Connects now, so a closed app or a wrong port shows at startup.
    pub fn open(target: Target) -> Result<Self, ControlError> {
        let mut link = Self { target, client: None };
        link.client = Some(link.connect()?);
        Ok(link)
    }

    fn connect(&self) -> Result<Client, ControlError> {
        match &self.target {
            Target::Running => Client::connect_running(),
            Target::Port(port) => Client::connect_published(SocketAddr::from((Ipv4Addr::LOCALHOST, *port))),
            Target::Address { addr, token } => Client::connect(*addr, token),
        }
    }

    /// Sends one request and returns the app's answer.
    ///
    /// Complexity: one round trip to the app, plus a reconnect when the last request lost it.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        let mut client = match self.client.take() {
            Some(client) => client,
            None => self.connect().map_err(|e| Failure::Unreachable(e.to_string()))?,
        };
        match client.call(method, params) {
            Ok(value) => {
                self.client = Some(client);
                Ok(value)
            }
            Err(ControlError::Failed { error, status }) => {
                self.client = Some(client);
                Err(match status.as_str() {
                    status::NOT_RUN => Failure::NotRun(error),
                    status::MAY_HAVE_RUN => Failure::MayHaveRun(error),
                    _ => Failure::Refused(error),
                })
            }
            // The connection is gone or out of step: the next request connects again.
            Err(other) => Err(Failure::Unreachable(other.to_string())),
        }
    }
}

/// An MCP server whose tools act on the app `target` names, with the `ui_*` tools added.
///
/// Does I/O: connects to the app.
pub fn connected_server(target: Target) -> Result<Server> {
    let mut link = Link::open(target).context("cannot reach the running Bodo app")?;
    let mut remote = Remote::new(move |method: &str, params: Value| link.request(method, params));
    let mut tools = bodo_mcp::tools();
    tools.extend(bodo_mcp::ui_tools());
    Ok(Server::with_backend("bodo-mcp-connected", tools, true, move |tool, args| remote.call(tool, args))?)
}

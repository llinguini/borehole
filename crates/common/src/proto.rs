// Borehole control protocol (v2)
//
// The control plane is a persistent TLS connection carrying newline-delimited
// JSON objects. Every frame is one JSON value terminated by '\n'. All messages
// are wrapped in the `Message` envelope: an internally-tagged enum whose
// `"type"` field selects the variant, so a single reader can decode any message
// arriving on the channel.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Tunnel protocol requested by the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Http,
}

/// Sent by the CLI to the server to open a tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Register {
    pub token: String,
    pub protocol: Protocol,
    pub local_port: u16,
    /// `None` lets the server assign a port from its pool.
    pub remote_port: Option<u16>,
    /// v2: preferred edge node name (e.g. "frankfurt"); `None` lets the server
    /// pick the least-loaded node (or the direct path when no node is up).
    #[serde(default)]
    pub node: Option<String>,
    /// v2: CLI host machine name, so the server can track devices. `None` from
    /// older CLIs.
    #[serde(default)]
    pub hostname: Option<String>,
}

/// Server reply after a successful `Register`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registered {
    pub remote_port: u16,
    /// v2: populated when the tunnel is hosted on a node; `None` in v1.
    pub node_host: Option<String>,
    pub node_port: Option<u16>,
}

/// Server -> CLI: an external connection arrived; open a data connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewConn {
    /// UUID v4 identifying the incoming connection.
    pub conn_id: String,
    /// v2: which node to open the `DataConn` against; `None` => the server.
    pub node_host: Option<String>,
    pub node_port: Option<u16>,
}

/// CLI -> server: a second TCP connection identifying which `conn_id` it serves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataConn {
    pub conn_id: String,
}

/// v2 — node -> server: a visitor reached the public port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitorConn {
    pub tunnel_id: String,
    pub conn_id: String,
}

/// v2 — node registration request to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterNode {
    pub token: String,
    /// Node name, e.g. "frankfurt".
    pub name: String,
    /// Plain-TCP port where the node accepts `DataConn` connections from CLIs.
    /// The server advertises it to CLIs (as `node_port`) so they dial the node
    /// directly for the data plane.
    pub data_port: u16,
}

/// v2 — server reply to a `RegisterNode`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistered {
    pub node_id: String,
}

/// v2 — server -> node: open a public tunnel listener for `tunnel_id` on
/// `remote_port`. The node binds the port and reports visitors via `VisitorConn`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenTunnel {
    pub tunnel_id: String,
    pub remote_port: u16,
}

/// v2 — server -> node: tear down `tunnel_id` (the owning CLI disconnected).
/// The node drops the public listener, freeing the port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseTunnel {
    pub tunnel_id: String,
}

/// Generic error, server -> client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMsg {
    pub reason: String,
}

/// Envelope wrapping every message exchanged on the control channel. The
/// `"type"` field (snake_case variant name) selects the payload, so any message
/// can be decoded without knowing it in advance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Register(Register),
    Registered(Registered),
    NewConn(NewConn),
    DataConn(DataConn),
    VisitorConn(VisitorConn),
    RegisterNode(RegisterNode),
    NodeRegistered(NodeRegistered),
    OpenTunnel(OpenTunnel),
    CloseTunnel(CloseTunnel),
    Error(ErrorMsg),
}

/// Serializes a message to JSON and appends a trailing '\n', producing a frame
/// ready to write to the control socket (newline-delimited JSON framing).
pub fn to_line(msg: &Message) -> Result<String> {
    let mut line = serde_json::to_string(msg).context("failed to serialize message")?;
    line.push('\n');
    Ok(line)
}

/// Deserializes a single JSON frame into a `Message`. A trailing newline (or
/// other surrounding whitespace) is tolerated.
pub fn from_line(line: &str) -> Result<Message> {
    serde_json::from_str(line.trim()).context("failed to deserialize message")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips a message through `to_line`/`from_line` and asserts the
    /// decoded value re-serializes identically. `Message` does not derive
    /// `PartialEq`, so we compare the canonical JSON instead.
    fn assert_roundtrip(msg: Message, expected_type: &str) {
        let line = to_line(&msg).expect("to_line must succeed");
        assert!(line.ends_with('\n'), "frame must end with a newline: {line:?}");
        assert!(
            line.contains(&format!(r#""type":"{expected_type}""#)),
            "missing type tag '{expected_type}' in: {line}"
        );

        let back = from_line(&line).expect("from_line must succeed");
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            serde_json::to_string(&back).unwrap(),
            "round-trip changed the message"
        );
    }

    #[test]
    fn register_roundtrips() {
        assert_roundtrip(
            Message::Register(Register {
                token: "secret".into(),
                protocol: Protocol::Tcp,
                local_port: 8080,
                remote_port: None,
                node: None,
                hostname: None,
            }),
            "register",
        );
    }

    #[test]
    fn registered_roundtrips() {
        assert_roundtrip(
            Message::Registered(Registered {
                remote_port: 32847,
                node_host: Some("fra1.example.com".into()),
                node_port: Some(40000),
            }),
            "registered",
        );
    }

    #[test]
    fn new_conn_roundtrips() {
        assert_roundtrip(
            Message::NewConn(NewConn {
                conn_id: "11111111-1111-4111-8111-111111111111".into(),
                node_host: None,
                node_port: None,
            }),
            "new_conn",
        );
    }

    #[test]
    fn data_conn_roundtrips() {
        assert_roundtrip(
            Message::DataConn(DataConn {
                conn_id: "abc-123".into(),
            }),
            "data_conn",
        );
    }

    #[test]
    fn visitor_conn_roundtrips() {
        assert_roundtrip(
            Message::VisitorConn(VisitorConn {
                tunnel_id: "tunnel-1".into(),
                conn_id: "abc-123".into(),
            }),
            "visitor_conn",
        );
    }

    #[test]
    fn register_node_roundtrips() {
        assert_roundtrip(
            Message::RegisterNode(RegisterNode {
                token: "node-token".into(),
                name: "frankfurt".into(),
                data_port: 7001,
            }),
            "register_node",
        );
    }

    #[test]
    fn node_registered_roundtrips() {
        assert_roundtrip(
            Message::NodeRegistered(NodeRegistered {
                node_id: "node-uuid".into(),
            }),
            "node_registered",
        );
    }

    #[test]
    fn error_roundtrips() {
        assert_roundtrip(
            Message::Error(ErrorMsg {
                reason: "invalid token".into(),
            }),
            "error",
        );
    }

    #[test]
    fn open_tunnel_roundtrips() {
        assert_roundtrip(
            Message::OpenTunnel(OpenTunnel {
                tunnel_id: "tunnel-1".into(),
                remote_port: 32847,
            }),
            "open_tunnel",
        );
    }

    #[test]
    fn close_tunnel_roundtrips() {
        assert_roundtrip(
            Message::CloseTunnel(CloseTunnel {
                tunnel_id: "tunnel-1".into(),
            }),
            "close_tunnel",
        );
    }

    #[test]
    fn protocol_serializes_lowercase() {
        let json = serde_json::to_string(&Protocol::Http).unwrap();
        assert_eq!(json, r#""http""#);
    }

    #[test]
    fn from_line_tolerates_trailing_newline() {
        let msg = from_line("{\"type\":\"data_conn\",\"conn_id\":\"x\"}\n")
            .expect("must parse with trailing newline");
        assert!(matches!(msg, Message::DataConn(_)));
    }
}

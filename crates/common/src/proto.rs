// Protocol message types
//
// Messages are exchanged as newline-delimited JSON objects over a TLS
// connection. Every message carries a `"type"` field acting as the
// discriminant, which maps to the `serde` internally-tagged enum below.

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// First message a client sends right after connecting. It authenticates the
/// client and describes the tunnel it wants to establish.
#[derive(Debug, Serialize, Deserialize)]
pub struct Register {
    /// Authentication token presented by the client.
    pub token: String,
    /// Tunnel protocol. Expected values: "tcp" or "http".
    pub protocol: String,
    /// Local port on the client side that traffic is forwarded to.
    pub local_port: u16,
    /// Desired public port on the server. `None` lets the server pick a
    /// random free port.
    pub remote_port: Option<u16>,
}

/// Sent by the client over a freshly opened second TCP connection to bind it
/// to a pending data stream previously announced by the server.
#[derive(Debug, Serialize, Deserialize)]
pub struct DataConn {
    /// Identifier (UUID v4) the server sent earlier in a `NewConn` message.
    pub conn_id: String,
}

/// Sent by the client to check connectivity and validate its token without
/// establishing a tunnel. The server answers with `Pong` on success or
/// `Error` if the token is rejected. Unlike `Register`, this acquires no port
/// and opens no public listener, so it is free of side effects.
#[derive(Debug, Serialize, Deserialize)]
pub struct Ping {
    /// Authentication token to validate against the server's token set.
    pub token: String,
}

/// Envelope for every message the client (CLI) sends to the server.
///
/// Serialized as an internally-tagged enum: the `"type"` field selects the
/// variant and its value is the snake_case form of the variant name.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Register(Register),
    DataConn(DataConn),
    Ping(Ping),
}

/// Confirms a successful `Register` and reports the public port bound to the
/// tunnel (useful when the client let the server pick a random one).
#[derive(Debug, Serialize, Deserialize)]
pub struct Registered {
    /// Public port the server assigned to the tunnel.
    pub remote_port: u16,
}

/// Notifies the client that an external connection reached the tunnel's public
/// port. The client is expected to open a data connection echoing `conn_id`.
#[derive(Debug, Serialize, Deserialize)]
pub struct NewConn {
    /// Unique identifier (UUID v4) for this incoming connection.
    pub conn_id: String,
}

/// Reports a server-side error, e.g. an invalid token.
#[derive(Debug, Serialize, Deserialize)]
pub struct ServerError {
    /// Human-readable explanation of the failure.
    pub reason: String,
}

/// Envelope for every message the server sends to the client (CLI).
///
/// Serialized as an internally-tagged enum: the `"type"` field selects the
/// variant and its value is the snake_case form of the variant name.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Registered(Registered),
    NewConn(NewConn),
    /// Successful reply to a `Ping`: connectivity and token are valid.
    Pong,
    #[serde(rename = "error")]
    Error(ServerError),
}

/// Serializes `msg` to JSON and appends a trailing '\n', producing a frame
/// ready to be written to the socket (newline-delimited JSON framing).
///
/// Generic over any `Serialize` type, so it is decoupled from the concrete
/// message enums.
pub fn encode<T: Serialize>(msg: &T) -> Result<String, serde_json::Error> {
    let mut framed = serde_json::to_string(msg)?;
    framed.push('\n');
    Ok(framed)
}

/// Deserializes a single line (with the trailing '\n' already stripped) into
/// the requested type `T`.
///
/// Generic over any `DeserializeOwned` type, so it is decoupled from the
/// concrete message enums.
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_serializes_with_type_tag() {
        let msg = ClientMsg::Register(Register {
            token: "secret".to_string(),
            protocol: "tcp".to_string(),
            local_port: 8080,
            remote_port: None,
        });

        let json = serde_json::to_string(&msg).expect("serialization must succeed");

        assert!(json.contains(r#""type":"register""#), "got: {json}");
    }

    #[test]
    fn data_conn_roundtrips() {
        let msg = ClientMsg::DataConn(DataConn {
            conn_id: "11111111-1111-4111-8111-111111111111".to_string(),
        });

        let json = serde_json::to_string(&msg).expect("serialization must succeed");
        let back: ClientMsg = serde_json::from_str(&json).expect("deserialization must succeed");

        assert!(matches!(back, ClientMsg::DataConn(_)));
    }

    #[test]
    fn registered_serializes_with_type_and_port() {
        let msg = ServerMsg::Registered(Registered { remote_port: 32847 });

        let json = serde_json::to_string(&msg).expect("serialization must succeed");

        assert!(json.contains(r#""type":"registered""#), "got: {json}");
        assert!(json.contains(r#""remote_port":32847"#), "got: {json}");
    }

    #[test]
    fn new_conn_deserializes() {
        let json = r#"{"type":"new_conn","conn_id":"abc-123"}"#;

        let msg: ServerMsg = serde_json::from_str(json).expect("deserialization must succeed");

        match msg {
            ServerMsg::NewConn(NewConn { conn_id }) => assert_eq!(conn_id, "abc-123"),
            other => panic!("expected NewConn, got: {other:?}"),
        }
    }

    #[test]
    fn error_deserializes() {
        let json = r#"{"type":"error","reason":"invalid token"}"#;

        let msg: ServerMsg = serde_json::from_str(json).expect("deserialization must succeed");

        match msg {
            ServerMsg::Error(ServerError { reason }) => assert_eq!(reason, "invalid token"),
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn ping_serializes_with_type_tag() {
        let msg = ClientMsg::Ping(Ping {
            token: "secret".to_string(),
        });

        let json = serde_json::to_string(&msg).expect("serialization must succeed");

        assert!(json.contains(r#""type":"ping""#), "got: {json}");
        assert!(json.contains(r#""token":"secret""#), "got: {json}");
    }

    #[test]
    fn pong_serializes_as_bare_tag() {
        let json = serde_json::to_string(&ServerMsg::Pong).expect("serialization must succeed");

        assert_eq!(json, r#"{"type":"pong"}"#);
    }

    #[test]
    fn pong_deserializes() {
        let msg: ServerMsg =
            serde_json::from_str(r#"{"type":"pong"}"#).expect("deserialization must succeed");

        assert!(matches!(msg, ServerMsg::Pong));
    }

    #[test]
    fn encode_decode_roundtrip() {
        let original = ClientMsg::Register(Register {
            token: "secret".to_string(),
            protocol: "http".to_string(),
            local_port: 3000,
            remote_port: Some(9000),
        });

        let framed = encode(&original).expect("encode must succeed");
        assert!(framed.ends_with('\n'), "frame must end with a newline");

        let line = framed.trim_end_matches('\n');
        let decoded: ClientMsg = decode(line).expect("decode must succeed");

        match (original, decoded) {
            (ClientMsg::Register(a), ClientMsg::Register(b)) => {
                assert_eq!(a.token, b.token);
                assert_eq!(a.protocol, b.protocol);
                assert_eq!(a.local_port, b.local_port);
                assert_eq!(a.remote_port, b.remote_port);
            }
            _ => panic!("expected a Register variant on both sides"),
        }
    }
}

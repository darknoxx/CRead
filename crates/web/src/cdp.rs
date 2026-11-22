//! The Chrome DevTools Protocol on a pipe: JSON messages, each ended by a NUL.
//!
//! Chromium reads commands on fd 3 and writes replies and events on fd 4
//! (`--remote-debugging-pipe`). There is no port, so no other process on the
//! machine can reach the browser.

use serde_json::{Value, json};

/// One outgoing command, framed for the pipe.
pub fn encode(id: u64, method: &str, params: Value, session: Option<&str>) -> Vec<u8> {
    let mut message = json!({ "id": id, "method": method, "params": params });
    if let Some(session) = session {
        message["sessionId"] = Value::from(session);
    }
    let mut bytes = message.to_string().into_bytes();
    bytes.push(0);
    bytes
}

/// Splits the incoming byte stream into messages.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    /// Feed what the pipe delivered; get every message it completed. A message
    /// that is not JSON is skipped rather than ending the session.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Value> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(end) = self.buf[start..].iter().position(|&b| b == 0) {
            if let Ok(value) = serde_json::from_slice(&self.buf[start..start + end]) {
                out.push(value);
            }
            start += end + 1;
        }
        self.buf.drain(..start);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_json_and_a_nul() {
        let bytes = encode(7, "Page.reload", json!({}), Some("S1"));
        assert_eq!(bytes.last(), Some(&0));
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "Page.reload");
        assert_eq!(value["sessionId"], "S1");
    }

    #[test]
    fn the_browser_session_has_no_session_id() {
        let bytes = encode(1, "Target.getTargets", json!({}), None);
        let value: Value = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
        assert!(value.get("sessionId").is_none());
    }

    #[test]
    fn messages_split_across_reads_come_out_whole() {
        let mut decoder = Decoder::default();
        assert!(decoder.push(b"{\"id\":1").is_empty());
        let out = decoder.push(b"}\0{\"id\":2}\0{\"id\"");
        assert_eq!(out.len(), 2);
        assert_eq!(out[1]["id"], 2);
        let out = decoder.push(b":3}\0");
        assert_eq!(out[0]["id"], 3);
    }

    #[test]
    fn garbage_is_skipped() {
        let mut decoder = Decoder::default();
        let out = decoder.push(b"nope\0{\"id\":4}\0");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], 4);
    }
}

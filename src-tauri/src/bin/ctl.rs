//! Tiny client for the running app. This is what the Hyprland binds execute:
//! starting a whole Tauri process per keypress would be far too slow.
//!
//! Usage: orra-ctl <start|stop|toggle|lang-next|lang-prev|lang|speak|open|status|quit> [text]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;

const DEFAULT_PORT: u16 = 47811;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().cloned().unwrap_or_else(|| "toggle".to_string());
    let rest = args.get(1..).unwrap_or(&[]).join(" ");

    let (port, token) = read_config();

    let stream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("orra is not running (127.0.0.1:{port}: {e})");
            std::process::exit(1);
        }
    };

    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("orra: {e}");
            std::process::exit(1);
        }
    };

    let mut request = format!("{token} {command}\n");

    // Commands that take an argument send it as a second line. The server reads
    // that line, so one must always follow — even when empty, as an empty
    // argument is a usage error it reports rather than something to wait on.
    let payload = match command.as_str() {
        // Fall back to stdin so the tray/keybind can pipe a whole transcript.
        "speak" if rest.is_empty() => {
            let mut buf = String::new();
            let _ = std::io::stdin().read_to_string(&mut buf);
            Some(buf)
        }
        "speak" | "lang" => Some(rest),
        _ => None,
    };
    if let Some(text) = payload {
        request.push_str(text.trim());
        request.push('\n');
    }

    if let Err(e) = writer.write_all(request.as_bytes()) {
        eprintln!("orra: {e}");
        std::process::exit(1);
    }

    let mut reply = String::new();
    let _ = BufReader::new(stream).read_line(&mut reply);
    let reply = reply.trim();
    if reply.starts_with("err") {
        eprintln!("orra: {reply}");
        std::process::exit(1);
    }
    // "ok" is the silent happy path; anything else carries information worth
    // printing (e.g. `status`), and a keybind discards stdout anyway.
    if !reply.is_empty() && reply != "ok" {
        println!("{reply}");
    }
}

/// Read just enough of the settings file to find the control port and token.
/// Deliberately dependency-free so this binary stays instant to start.
fn read_config() -> (u16, String) {
    let Ok(body) = std::fs::read_to_string(config_path()) else {
        return (DEFAULT_PORT, String::new());
    };
    let port = json_number(&body, "port").unwrap_or(DEFAULT_PORT as u64) as u16;
    let token = json_string(&body, "token").unwrap_or_default();
    (port, token)
}

fn config_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("orra/config.json");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".config/orra/config.json")
}

/// Pull `"key": <digits>` out of a flat JSON object.
fn json_number(body: &str, key: &str) -> Option<u64> {
    let at = body.find(&format!("\"{key}\""))?;
    let after = &body[at + key.len() + 2..];
    let start = after.find(':')? + 1;
    let digits: String = after[start..].trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Pull `"key": "value"` out of a flat JSON object.
fn json_string(body: &str, key: &str) -> Option<String> {
    let at = body.find(&format!("\"{key}\""))?;
    let after = &body[at + key.len() + 2..];
    let start = after.find(':')? + 1;
    let after = after[start..].trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_json_helpers_pick_out_port_and_token() {
        let body = r#"{
  "hotkey": "SUPER + ALT + D",
  "port": 47811,
  "token": "abc123",
  "api_key": ""
}"#;
        assert_eq!(json_number(body, "port"), Some(47811));
        assert_eq!(json_string(body, "token").as_deref(), Some("abc123"));
        assert_eq!(json_string(body, "missing"), None);
    }
}

//! SSH agent and key discovery, for the settings panel. Never reads key material.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SshStatus {
    pub agent_socket: bool,
    /// Comments of identities loaded in the agent (from `ssh-add -l`).
    pub loaded_identities: Vec<String>,
    /// Private key files found in `~/.ssh` (names only).
    pub key_files: Vec<PathBuf>,
}

pub fn status() -> SshStatus {
    let agent_socket = std::env::var_os("SSH_AUTH_SOCK").is_some();
    let loaded_identities = if agent_socket {
        Command::new("ssh-add")
            .arg("-l")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| {
                        // "256 SHA256:... comment (ED25519)" -> comment
                        let mut parts = l.split_whitespace();
                        let _bits = parts.next();
                        let _fp = parts.next();
                        let rest: Vec<&str> = parts.collect();
                        rest.join(" ")
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let key_files = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".ssh"))
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map(|entries| {
            let mut v: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    p.is_file()
                        && name.starts_with("id_")
                        && !name.ends_with(".pub")
                        && !name.ends_with("-cert.pub")
                })
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default();
    SshStatus {
        agent_socket,
        loaded_identities,
        key_files,
    }
}

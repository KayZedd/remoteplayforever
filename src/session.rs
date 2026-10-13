//! Parent side of the Remote Play Together session: spawns the out-of-process host
//! (`remoteplayforever.exe --host-session`, see `rpt.rs`) and talks to it over a line protocol.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::i18n::tr;

/// Every protocol line the host prints starts with this, so log noise on stdout is ignored.
pub const PREFIX: &str = "@RPT ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Keyboard,
    Mouse,
    Controller,
}

/// What guests are allowed to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestInputs {
    pub keyboard: bool,
    pub mouse: bool,
    pub controller: bool,
}

/// Host -> parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Status(String),
    Link(String),
    GuestConnected,
    Player { slot: u32, steam: u64, name: String },
    Error(String),
    Ready,
    Stopped,
}

/// Parent -> host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCommand {
    AddLink,
    SetInput(InputKind, bool),
    Stop,
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

impl HostEvent {
    pub fn to_line(&self) -> String {
        let body = match self {
            HostEvent::Status(s) => format!("STATUS {}", one_line(s)),
            HostEvent::Link(u) => format!("LINK {}", one_line(u)),
            HostEvent::GuestConnected => "GUEST_CONNECTED".to_string(),
            HostEvent::Player { slot, steam, name } => format!("PLAYER {slot} {steam} {}", one_line(name)),
            HostEvent::Error(s) => format!("ERROR {}", one_line(s)),
            HostEvent::Ready => "READY".to_string(),
            HostEvent::Stopped => "STOPPED".to_string(),
        };
        format!("{PREFIX}{body}")
    }

    pub fn parse(line: &str) -> Option<Self> {
        let body = line.trim_end_matches(['\r', '\n']).strip_prefix(PREFIX)?;
        let (kind, rest) = body.split_once(' ').unwrap_or((body, ""));
        match kind {
            "STATUS" => Some(HostEvent::Status(rest.to_string())),
            "LINK" if !rest.is_empty() => Some(HostEvent::Link(rest.to_string())),
            "GUEST_CONNECTED" => Some(HostEvent::GuestConnected),
            "PLAYER" => {
                let mut it = rest.splitn(3, ' ');
                let slot = it.next()?.parse().ok()?;
                let steam = it.next()?.parse().ok()?;
                let name = it.next().unwrap_or("").to_string();
                Some(HostEvent::Player { slot, steam, name })
            }
            "ERROR" => Some(HostEvent::Error(rest.to_string())),
            "READY" => Some(HostEvent::Ready),
            "STOPPED" => Some(HostEvent::Stopped),
            _ => None,
        }
    }
}

impl HostCommand {
    pub fn to_line(&self) -> String {
        match self {
            HostCommand::AddLink => "ADD".to_string(),
            HostCommand::Stop => "STOP".to_string(),
            HostCommand::SetInput(kind, on) => {
                let k = match kind {
                    InputKind::Keyboard => "KBD",
                    InputKind::Mouse => "MOUSE",
                    InputKind::Controller => "CTRL",
                };
                format!("{k} {}", u8::from(*on))
            }
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let mut it = line.split_whitespace();
        let kind = it.next()?.to_ascii_uppercase();
        let on = || it.clone().next().map(|v| v == "1");
        match kind.as_str() {
            "ADD" => Some(HostCommand::AddLink),
            "STOP" => Some(HostCommand::Stop),
            "KBD" => Some(HostCommand::SetInput(InputKind::Keyboard, on()?)),
            "MOUSE" => Some(HostCommand::SetInput(InputKind::Mouse, on()?)),
            "CTRL" => Some(HostCommand::SetInput(InputKind::Controller, on()?)),
            _ => None,
        }
    }
}

/// A running session host process. Dropping it ends the session.
pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    /// Disconnects when the host process exits.
    pub events: Receiver<HostEvent>,
}

impl Session {
    pub fn start(appid: u32, desktop_stream: bool, inputs: GuestInputs) -> Result<Self> {
        let exe = std::env::current_exe().context("current_exe")?;
        let mut cmd = Command::new(exe);
        cmd.arg("--host-session").arg(appid.to_string());
        if desktop_stream {
            cmd.arg("--desktop-stream");
        }
        for (allowed, flag) in [(inputs.keyboard, "--no-keyboard"), (inputs.mouse, "--no-mouse"), (inputs.controller, "--no-controller")] {
            if !allowed {
                cmd.arg(flag);
            }
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().context(tr("Could not start the Remote Play session host", "Nie udało się uruchomić hosta sesji Remote Play"))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().context("session host has no stdout")?;

        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(ev) = HostEvent::parse(&line) {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            }
        });

        Ok(Self { child, stdin, events })
    }

    pub fn send(&mut self, cmd: HostCommand) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{}", cmd.to_line());
            let _ = stdin.flush();
        }
    }

    /// Ask the host to disband the group, then make sure the process is gone.
    pub fn stop(&mut self) {
        self.send(HostCommand::Stop);
        self.stdin = None; // EOF also tells the host to tear down
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_roundtrip() {
        let events = [
            HostEvent::Status("Creating the group".into()),
            HostEvent::Link("https://s.team/remoteplay/connect/abc".into()),
            HostEvent::GuestConnected,
            HostEvent::Player { slot: 2, steam: 76561198000000001, name: "Player 2".into() },
            HostEvent::Error("Steam is not running".into()),
            HostEvent::Ready,
            HostEvent::Stopped,
        ];
        for ev in events {
            assert_eq!(HostEvent::parse(&ev.to_line()), Some(ev));
        }
    }

    #[test]
    fn event_parse_ignores_log_noise() {
        assert_eq!(HostEvent::parse("LINK https://s.team/x"), None);
        assert_eq!(HostEvent::parse("rpt: something happened"), None);
        assert_eq!(HostEvent::parse("@RPT LINK"), None);
        assert_eq!(HostEvent::parse("@RPT WHATEVER 1"), None);
    }

    #[test]
    fn event_is_always_one_line() {
        let line = HostEvent::Error("a\r\nb".into()).to_line();
        assert!(!line.contains('\n') && !line.contains('\r'));
    }

    #[test]
    fn command_roundtrip() {
        let cmds = [
            HostCommand::AddLink,
            HostCommand::Stop,
            HostCommand::SetInput(InputKind::Keyboard, true),
            HostCommand::SetInput(InputKind::Mouse, false),
            HostCommand::SetInput(InputKind::Controller, true),
        ];
        for cmd in cmds {
            assert_eq!(HostCommand::parse(&cmd.to_line()), Some(cmd));
        }
        assert_eq!(HostCommand::parse("KBD"), None);
        assert_eq!(HostCommand::parse("nonsense 1"), None);
    }
}

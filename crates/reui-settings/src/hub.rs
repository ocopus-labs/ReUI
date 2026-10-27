//! Telling the running hub about a change: connect to its pipe as a Settings surface, send
//! module commands (`reload` after a write, `clear`), hang up. On a thread, so a busy or
//! missing hub never stalls the window; when ReUI isn't running the files alone are enough,
//! since every module reads its file when it starts.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::time::Duration;

use reui_protocol::{pipe_name, ClientMsg, HubMsg, SurfaceKind, PROTOCOL_VERSION};

const ERROR_PIPE_BUSY: i32 = 231;
/// Lets the hub read the commands before the pipe closes under it.
const LINGER: Duration = Duration::from_millis(150);

/// Sends `(module, action)` commands to the hub, in order, in the background.
pub fn send(commands: Vec<(String, String)>) {
    if commands.is_empty() {
        return;
    }
    _ = std::thread::Builder::new().name("hub".into()).spawn(move || {
        if let Err(e) = send_now(&commands) {
            if e.kind() != io::ErrorKind::NotFound {
                eprintln!("settings: couldn't reach ReUI: {e}");
            }
        }
    });
}

fn send_now(commands: &[(String, String)]) -> io::Result<()> {
    let mut pipe = open()?;
    write_frame(&mut pipe, &ClientMsg::Hello { version: PROTOCOL_VERSION, surface: SurfaceKind::Settings })?;
    match read_frame(&mut pipe)? {
        HubMsg::Welcome { .. } => {}
        HubMsg::Rejected { reason } => return Err(io::Error::other(reason)),
        other => return Err(io::Error::other(format!("unexpected {other:?}"))),
    }
    for (module, action) in commands {
        write_frame(&mut pipe, &ClientMsg::Command { module: module.clone(), action: action.clone() })?;
    }
    std::thread::sleep(LINGER);
    Ok(())
}

fn open() -> io::Result<File> {
    for _ in 0..25 {
        match OpenOptions::new().read(true).write(true).open(pipe_name()) {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => std::thread::sleep(Duration::from_millis(20)),
            other => return other,
        }
    }
    Err(io::Error::other("the hub's pipe stayed busy"))
}

/// The protocol's framing (`reui_protocol::write_frame`), without an async runtime.
fn write_frame(w: &mut impl Write, msg: &ClientMsg) -> io::Result<()> {
    let body = postcard::to_stdvec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    w.write_all(&frame)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> io::Result<HubMsg> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 1 << 20 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    postcard::from_bytes(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_match_the_protocols() {
        let msg = ClientMsg::Command { module: "media".into(), action: "reload".into() };
        let mut ours = Vec::new();
        write_frame(&mut ours, &msg).unwrap();
        let body = postcard::to_stdvec(&msg).unwrap();
        assert_eq!(&ours[..4], &(body.len() as u32).to_le_bytes());
        assert_eq!(&ours[4..], &body[..]);

        let welcome = postcard::to_stdvec(&HubMsg::Welcome { version: PROTOCOL_VERSION }).unwrap();
        let mut frame = (welcome.len() as u32).to_le_bytes().to_vec();
        frame.extend(welcome);
        assert_eq!(read_frame(&mut &frame[..]).unwrap(), HubMsg::Welcome { version: PROTOCOL_VERSION });
    }
}

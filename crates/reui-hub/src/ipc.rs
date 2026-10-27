//! Named-pipe server for surfaces. The pipe only admits the current user, rejects remote
//! clients, and is created as the first instance so another process can't squat the name.

use std::collections::HashSet;
use std::ffi::c_void;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reui_protocol::{read_frame, write_frame, ClientMsg, HubMsg, ImageId, Snapshot, SurfaceKind, PROTOCOL_VERSION};
use tokio::io::{AsyncReadExt, ReadHalf, WriteHalf};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::sync::oneshot;
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::hub::Hub;

/// At most one snapshot per frame per surface; bursts in between collapse into the latest.
const MIN_SEND_INTERVAL: Duration = Duration::from_millis(16);

pub struct Server {
    name: String,
    /// Owned for the life of the process; stored as an address so the server stays `Send`.
    descriptor: usize,
    listening: NamedPipeServer,
}

impl Server {
    pub fn bind(name: &str) -> Result<Self> {
        let descriptor = current_user_only_descriptor()?;
        let listening = create(name, descriptor, true).context("another ReUI hub is already running for this user")?;
        Ok(Self { name: name.to_string(), descriptor, listening })
    }

    pub async fn run(mut self, hub: Hub) {
        loop {
            if let Err(e) = self.listening.connect().await {
                eprintln!("ipc: connect failed: {e}");
                continue;
            }
            let next = match create(&self.name, self.descriptor, false) {
                Ok(next) => next,
                Err(e) => {
                    eprintln!("ipc: can't create next pipe instance: {e}");
                    return;
                }
            };
            let connection = std::mem::replace(&mut self.listening, next);
            let hub = hub.clone();
            tokio::spawn(async move {
                if let Err(e) = serve(connection, hub).await {
                    eprintln!("ipc: surface disconnected: {e}");
                }
            });
        }
    }
}

fn create(name: &str, descriptor: usize, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor as *mut c_void,
        bInheritHandle: false.into(),
    };
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).reject_remote_clients(true).pipe_mode(PipeMode::Byte);
    unsafe { options.create_with_security_attributes_raw(name, &mut attributes as *mut _ as *mut c_void) }
}

async fn serve(pipe: NamedPipeServer, hub: Hub) -> Result<()> {
    let (mut reader, mut writer) = tokio::io::split(pipe);
    let surface = match read_frame::<_, ClientMsg>(&mut reader).await? {
        ClientMsg::Hello { version, surface } if version == PROTOCOL_VERSION => surface,
        ClientMsg::Hello { version, .. } => {
            let reason = format!("hub speaks protocol {PROTOCOL_VERSION}, surface speaks {version}");
            write_frame(&mut writer, &HubMsg::Rejected { reason: reason.clone() }).await?;
            bail!(reason);
        }
        other => bail!("expected Hello, got {other:?}"),
    };
    write_frame(&mut writer, &HubMsg::Welcome { version: PROTOCOL_VERSION }).await?;
    if surface == SurfaceKind::Bridge {
        return serve_bridge(reader, writer, hub).await;
    }
    println!("ipc: {surface:?} connected");

    let mut snapshots = hub.subscribe();
    let mut focus = hub.subscribe_focus();
    let images = hub.clone();
    let sender = tokio::spawn(async move {
        let mut sent = HashSet::new();
        // Snapshot (with its images not yet sent to this surface) and focus requests share
        // the writer; a focus always follows the snapshot that contains its activity.
        macro_rules! send_snapshot {
            () => {{
                let snapshot = snapshots.borrow_and_update().as_ref().clone();
                for id in images_to_send(&mut sent, &snapshot) {
                    // An image the hub no longer has isn't marked sent, so it goes out
                    // if it comes back.
                    if let Some(bytes) = images.image(id) {
                        if write_frame(&mut writer, &HubMsg::Image { id, bytes: bytes.to_vec() }).await.is_err() {
                            return;
                        }
                        sent.insert(id);
                    }
                }
                if write_frame(&mut writer, &HubMsg::Snapshot(snapshot)).await.is_err() {
                    return;
                }
            }};
        }
        send_snapshot!();
        loop {
            tokio::select! {
                changed = snapshots.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    tokio::time::sleep(MIN_SEND_INTERVAL).await;
                    send_snapshot!();
                }
                request = focus.recv() => match request {
                    Ok(id) => {
                        if snapshots.has_changed().unwrap_or(false) {
                            send_snapshot!();
                        }
                        if write_frame(&mut writer, &HubMsg::Focus { id }).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => return,
                },
            }
        }
    });

    let result = loop {
        match read_frame::<_, ClientMsg>(&mut reader).await {
            Ok(ClientMsg::Action { id, action }) => hub.route_action(&id, action),
            Ok(ClientMsg::Command { module, action }) => hub.route_command(&module, action),
            Ok(ClientMsg::Hello { .. } | ClientMsg::Hook { .. }) => {}
            Err(e) => break Err(e.into()),
        }
    };
    sender.abort();
    println!("ipc: {surface:?} disconnected");
    result
}

/// The images `snapshot` shows that this surface doesn't have yet, in order. `sent` keeps
/// only what the snapshot shows: the island drops its decoded copies of everything else at
/// the same point in the stream (`reui-island` `panel::referenced`), so an image that comes
/// back later is simply sent again.
fn images_to_send(sent: &mut HashSet<ImageId>, snapshot: &Snapshot) -> Vec<ImageId> {
    let shown: HashSet<ImageId> = snapshot.activities.iter().flat_map(|a| a.images()).collect();
    sent.retain(|id| shown.contains(id));
    let mut missing = Vec::new();
    for id in snapshot.activities.iter().flat_map(|a| a.images()) {
        if !sent.contains(&id) && !missing.contains(&id) {
            missing.push(id);
        }
    }
    missing
}

/// One hook call. If the agent waits for an answer, hold the connection until the module
/// replies, or until the bridge disconnects (the agent answered elsewhere or gave up), in
/// which case the module is told so it can drop the request from the island.
async fn serve_bridge(mut reader: ReadHalf<NamedPipeServer>, mut writer: WriteHalf<NamedPipeServer>, hub: Hub) -> Result<()> {
    let ClientMsg::Hook { agent, payload, wants_reply } = read_frame::<_, ClientMsg>(&mut reader).await? else {
        bail!("bridge sent something other than a hook");
    };
    if !wants_reply {
        hub.route_hook(&agent, payload, None);
        return Ok(());
    }
    let (tx, rx) = oneshot::channel();
    let Some(call) = hub.route_hook(&agent, payload, Some(tx)) else {
        write_frame(&mut writer, &HubMsg::HookReply { output: None }).await?;
        return Ok(());
    };
    let mut probe = [0u8; 1];
    tokio::select! {
        output = rx => write_frame(&mut writer, &HubMsg::HookReply { output: output.ok().flatten() }).await?,
        // The bridge never sends more, so any read completing means it went away.
        _ = reader.read(&mut probe) => hub.hook_gone(&agent, call),
    }
    Ok(())
}

/// A security descriptor whose DACL grants access to the current user's SID only.
fn current_user_only_descriptor() -> Result<usize> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let mut len = 0u32;
        _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buffer = vec![0u8; len as usize];
        let info = GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr() as *mut c_void), len, &mut len);
        _ = CloseHandle(token);
        info?;
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);

        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut sid)?;
        let sid_string = sid.to_string();
        _ = LocalFree(Some(HLOCAL(sid.0 as *mut c_void)));

        let sddl = HSTRING::from(format!("D:P(A;;GA;;;{})", sid_string?));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut descriptor, None)?;
        Ok(descriptor.0 as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reui_protocol::{Activity, ActivityId, Art, CompactView, Element, Priority};

    fn showing(ids: &[u64]) -> Snapshot {
        let activity = |i: &u64| Activity {
            id: ActivityId { module: "m".into(), key: i.to_string() },
            priority: Priority::Ongoing,
            accent: None,
            compact: CompactView { leading: Element::Art(Art::Image(ImageId(*i))), trailing: Element::Empty },
            minimal: Element::Art(Art::Image(ImageId(*i))),
            audio: None,
            expanded: None,
        };
        Snapshot { activities: ids.iter().map(activity).collect() }
    }

    #[test]
    fn images_are_sent_once_and_again_after_they_were_dropped() {
        let mut sent = HashSet::new();
        let mut send = |ids: &[u64]| {
            let out = images_to_send(&mut sent, &showing(ids));
            sent.extend(&out);
            out.into_iter().map(|i| i.0).collect::<Vec<_>>()
        };
        assert_eq!(send(&[1, 2]), [1, 2]);
        assert_eq!(send(&[1, 2]), Vec::<u64>::new());
        // 1 leaves the snapshot: both ends drop it, so it's sent again when it's back.
        assert_eq!(send(&[2]), Vec::<u64>::new());
        assert_eq!(send(&[1, 2]), [1]);
    }
}

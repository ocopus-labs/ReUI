//! "The internet is back" (PRD §6.5): Windows' connectivity hints
//! (`NotifyNetworkConnectivityHintChange`, Windows 10 2004+) so a feed that failed offline
//! is fetched the moment a network with internet access comes up, rather than at its next
//! retry. Event-driven: the callback runs on a system thread pool only when the hint
//! changes, and nothing polls.

use std::ffi::c_void;

use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::Foundation::{HANDLE, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{CancelMibChangeNotify2, NotifyNetworkConnectivityHintChange};
use windows::Win32::Networking::WinSock::{NetworkConnectivityLevelHintInternetAccess, NL_NETWORK_CONNECTIVITY_HINT};

/// Registered while alive; dropping it stops the notifications.
pub struct Watch {
    handle: HANDLE,
    sender: *mut UnboundedSender<()>,
}

// The handle and the boxed sender are only touched on registration and drop.
unsafe impl Send for Watch {}

/// Sends `()` each time the PC gains internet access.
pub fn watch(online: UnboundedSender<()>) -> Option<Watch> {
    let sender = Box::into_raw(Box::new(online));
    let mut handle = HANDLE::default();
    let result = unsafe { NotifyNetworkConnectivityHintChange(Some(changed), Some(sender as *const c_void), false, &mut handle) };
    if result != NO_ERROR {
        eprintln!("calendar: no connectivity notifications ({})", result.0);
        drop(unsafe { Box::from_raw(sender) });
        return None;
    }
    Some(Watch { handle, sender })
}

unsafe extern "system" fn changed(context: *const c_void, hint: NL_NETWORK_CONNECTIVITY_HINT) {
    if hint.ConnectivityLevel == NetworkConnectivityLevelHintInternetAccess {
        if let Some(sender) = unsafe { (context as *const UnboundedSender<()>).as_ref() } {
            _ = sender.send(());
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // Returns once no callback is running, so the sender can go.
        unsafe {
            _ = CancelMibChangeNotify2(self.handle);
            drop(Box::from_raw(self.sender));
        }
    }
}

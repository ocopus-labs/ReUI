//! The worker thread: decodes copies, builds every format for a write and hands it to the
//! window thread, then types the paste.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::DROPFILES;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::access::{html_from_clipboard, html_to_clipboard};
use super::formats::dib_from_bgra;
use super::image::{app_icon, dib_image, png_image, png_to_bgra};
use super::target::{app_name, file_name_lower};
use super::window::WM_WRITE_READY;
use super::{paste, Captured, Content, Formats, Job, OsEvent, Raw, Write};

/// The slow half: image codecs, icons, paste prep and key input, off the window thread.
pub(super) struct Worker {
    events: UnboundedSender<OsEvent>,
    writes: std_mpsc::Sender<Write>,
    hwnd: isize,
    formats: Formats,
    wic: Option<IWICImagingFactory>,
    /// App icons by executable path (icons don't change while we run).
    icons: HashMap<String, Option<Vec<u8>>>,
}

impl Worker {
    pub(super) fn new(events: UnboundedSender<OsEvent>, writes: std_mpsc::Sender<Write>, hwnd: isize, formats: Formats) -> Worker {
        let wic = unsafe {
            _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
        };
        Worker { events, writes, hwnd, formats, wic, icons: HashMap::new() }
    }

    pub(super) fn run(mut self, jobs: std_mpsc::Receiver<Job>) {
        // A panic on one odd copy (a malformed image, say) must not end capture: this thread
        // carries every copy and paste, and losing it would silently stop both.
        while let Ok(job) = jobs.recv() {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(job))).is_err() {
                eprintln!("clipboard: handling a copy or paste failed unexpectedly; skipped it");
            }
        }
    }

    fn handle(&mut self, job: Job) {
        {
            match job {
                Job::Copied { raw, owner } => {
                    if let Some(captured) = self.captured(raw, owner) {
                        _ = self.events.send(OsEvent::Copied(captured));
                    }
                }
                Job::Write { content, plain, target } => match self.put(&content, plain) {
                    Ok(()) => {
                        if let Some(target) = target {
                            paste::type_paste(&self.events, HWND(target as *mut c_void));
                        }
                    }
                    Err(e) => eprintln!("clipboard: can't put the item back: {e}"),
                },
            }
        }
    }

    fn captured(&mut self, raw: Raw, owner: Option<String>) -> Option<Captured> {
        let mut size = None;
        let content = match raw {
            Raw::Text { utf16, html, rtf } => {
                let units: Vec<u16> = utf16.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                Content::Text { text: String::from_utf16_lossy(&units), html: html.as_deref().and_then(html_from_clipboard), rtf }
            }
            Raw::Files { paths, cut } => Content::Files { paths, cut },
            Raw::Png(_) | Raw::Dib(_) => {
                let wic = self.wic.as_ref()?;
                let image = match raw {
                    Raw::Png(png) => unsafe { png_image(wic, png) },
                    Raw::Dib(dib) => unsafe { dib_image(wic, &dib) },
                    _ => unreachable!(),
                };
                match image {
                    Ok((content, w, h)) => {
                        size = Some((w, h));
                        content
                    }
                    Err(e) => {
                        eprintln!("clipboard: can't read the copied image: {e}");
                        return None;
                    }
                }
            }
        };
        let icon = owner.as_deref().and_then(|exe| {
            let wic = self.wic.clone()?;
            self.icons.entry(exe.to_string()).or_insert_with(|| app_icon(&wic, exe)).clone()
        });
        let exe = owner.as_deref().map(file_name_lower);
        Some(Captured { content, source: owner.as_deref().map(app_name), exe, icon, size })
    }

    /// Decodes and builds every format first, then has the window thread set them; a busy
    /// clipboard gets a few retries here, where sleeping blocks nothing.
    fn put(&self, content: &Content, plain: bool) -> std::result::Result<(), String> {
        let data = Arc::new(self.prepare(content, plain).map_err(|e| e.to_string())?);
        let mut last = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            let (reply, result) = std_mpsc::channel();
            self.writes.send(Write { data: data.clone(), reply }).map_err(|_| "the clipboard thread is gone")?;
            unsafe { PostMessageW(Some(HWND(self.hwnd as *mut c_void)), WM_WRITE_READY, WPARAM(0), LPARAM(0)) }.map_err(|e| e.to_string())?;
            match result.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(e)) => last = e,
                Err(_) => return Err("the clipboard thread didn't answer".into()),
            }
        }
        Err(format!("the clipboard stayed busy ({last})"))
    }

    /// Every format, most descriptive first (apps take the first one they understand).
    fn prepare(&self, content: &Content, plain: bool) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut data = Vec::new();
        let mut push = |format: u32, bytes: Vec<u8>| {
            if format != 0 {
                data.push((format, bytes));
            }
        };
        match content {
            Content::Text { text, html, rtf } => {
                if !plain {
                    if let Some(html) = html {
                        push(self.formats.html, html_to_clipboard(html));
                    }
                    if let Some(rtf) = rtf {
                        push(self.formats.rtf, rtf.iter().copied().chain([0]).collect());
                    }
                }
                let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
                push(u32::from(CF_UNICODETEXT.0), bytemuck_u16(&units).to_vec());
            }
            Content::Files { paths, .. } => {
                let header = DROPFILES { pFiles: size_of::<DROPFILES>() as u32, pt: POINT::default(), fNC: false.into(), fWide: true.into() };
                let mut bytes = unsafe { std::slice::from_raw_parts(&header as *const DROPFILES as *const u8, size_of::<DROPFILES>()) }.to_vec();
                for file in paths {
                    let units: Vec<u16> = file.encode_utf16().chain([0]).collect();
                    bytes.extend_from_slice(bytemuck_u16(&units));
                }
                bytes.extend_from_slice(&[0, 0]);
                push(u32::from(CF_HDROP.0), bytes);
                // Always a copy: replaying a cut would move the files out from under the history.
                push(self.formats.drop_effect, 1u32.to_le_bytes().to_vec());
            }
            Content::Image { png, .. } => {
                let wic = self.wic.as_ref().ok_or_else(windows::core::Error::empty)?;
                let (w, h, bgra) = unsafe { png_to_bgra(wic, png)? };
                push(self.formats.png, png.clone());
                push(u32::from(CF_DIBV5.0), dib_from_bgra(w, h, &bgra, true));
                push(u32::from(CF_DIB.0), dib_from_bgra(w, h, &bgra, false));
            }
        }
        // Keep the paste-back out of Win+V history and other clipboard managers (and us).
        push(self.formats.history, 0u32.to_le_bytes().to_vec());
        push(self.formats.exclude, vec![0]);
        Ok(data)
    }

}

fn bytemuck_u16(units: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2) }
}

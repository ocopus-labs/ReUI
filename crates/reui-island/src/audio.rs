//! The real visualizer: per-process loopback capture of the playing app, folded into four
//! band levels for the four bars. Runs only while bars are live and shown; any trouble
//! (activation fails, the process makes no sound, a DRM or exclusive path yields silence)
//! means the pill keeps its synthetic bars. Audio never leaves this process's memory, and
//! only the band levels outlive each analysis window.
//!
//! Capture runs on its own MTA thread (process-loopback activation completes on an MTA)
//! and stops by itself within [`POLL_MS`] of being dropped. The UI thread only reads the
//! latest levels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use realfft::{RealFftPlanner, RealToComplex};
use reui_design::tokens::motion;
use windows::core::{Interface, Ref, Result, HRESULT};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, BLOB, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;

pub const BANDS: usize = 4;
const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// FFT window: ~21 ms at 48 kHz, 47 Hz per bin, enough to separate the lowest band.
const WINDOW: usize = 1024;
/// New frames between analyses (60 a second), so attack stays crisp at the bars' 30 fps.
const HOP: usize = 800;
/// Band edges in Hz, log-spaced: bass, low mids, high mids, presence.
const EDGES: [f32; BANDS + 1] = [60.0, 250.0, 1_000.0, 4_000.0, 12_000.0];
/// Music carries less energy up high; lift those bands so all four move.
const TILT_DB: [f32; BANDS] = [0.0, 4.0, 8.0, 12.0];
/// Band level in dBFS mapped to an empty and a full bar.
const FLOOR_DB: f32 = -60.0;
const CEIL_DB: f32 = -12.0;
/// Below this a sample counts as digital silence.
const SILENT: f32 = 1e-5;
/// This much silence (or no audio at all) while "playing" hands the bars back to the
/// synthetic ones (E-A5).
const SILENCE_LIMIT: Duration = Duration::from_secs(2);
/// How often the capture thread checks it's still wanted when no audio arrives.
const POLL_MS: u32 = 100;
/// Activation normally completes in a few ms.
const ACTIVATE_TIMEOUT_MS: u32 = 3_000;

/// Turns mono samples into smoothed, dB-normalised band levels (0..=1).
pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// The latest `WINDOW` samples, a ring; `head` is the oldest.
    history: Vec<f32>,
    head: usize,
    fresh: usize,
    input: Vec<f32>,
    spectrum: Vec<realfft::num_complex::Complex<f32>>,
    bins: [std::ops::Range<usize>; BANDS],
    pub levels: [f32; BANDS],
    /// Frames since the last audible sample; `None` until anything was heard.
    pub silent_frames: Option<usize>,
}

impl Default for Analyzer {
    fn default() -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        let hz_per_bin = RATE as f32 / WINDOW as f32;
        let bin = |hz: f32| (hz / hz_per_bin).ceil() as usize;
        let bins = std::array::from_fn(|b| bin(EDGES[b])..bin(EDGES[b + 1]));
        // Hann window: keeps a tone's energy in its own band.
        let window = (0..WINDOW).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos()).collect();
        Self {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            fft,
            window,
            history: vec![0.0; WINDOW],
            head: 0,
            fresh: 0,
            bins,
            levels: [0.0; BANDS],
            silent_frames: None,
        }
    }
}

impl Analyzer {
    /// Adds mono samples; analyses every [`HOP`] new ones.
    pub fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.history[self.head] = s;
            self.head = (self.head + 1) % WINDOW;
            self.silent_frames = match self.silent_frames {
                _ if s.abs() > SILENT => Some(0),
                Some(n) => Some(n + 1),
                None => None,
            };
            self.fresh += 1;
            if self.fresh >= HOP {
                self.fresh = 0;
                self.analyse();
            }
        }
    }

    /// Heard something, and not silent for too long since.
    pub fn audible(&self) -> bool {
        self.silent_frames.is_some_and(|n| (n as u64) < SILENCE_LIMIT.as_millis() as u64 * RATE as u64 / 1000)
    }

    fn analyse(&mut self) {
        let (newer, older) = self.history.split_at(self.head);
        for ((x, s), w) in self.input.iter_mut().zip(older.iter().chain(newer)).zip(&self.window) {
            *x = s * w;
        }
        if self.fft.process(&mut self.input, &mut self.spectrum).is_err() {
            return;
        }
        // A full-scale sine peaks at WINDOW / 4 through a Hann window: that's 0 dBFS.
        let norm = 4.0 / WINDOW as f32;
        let dt = HOP as f32 / RATE as f32 * 1000.0;
        for (b, bins) in self.bins.iter().enumerate() {
            let power: f32 = self.spectrum[bins.clone()].iter().map(|c| (c.norm() * norm).powi(2)).sum();
            let db = 10.0 * power.max(1e-12).log10() + TILT_DB[b];
            let target = ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0);
            // Fast attack, slower release, independent of the hop.
            let tau = if target > self.levels[b] { motion::bars::ATTACK } else { motion::bars::RELEASE } as f32;
            self.levels[b] += (target - self.levels[b]) * (1.0 - (-dt / tau.max(1.0)).exp());
        }
    }
}

/// What the capture thread publishes.
#[derive(Default)]
struct Published {
    levels: [f32; BANDS],
    audible: bool,
    at: Option<Instant>,
}

#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    latest: Mutex<Published>,
}

struct Capture {
    pid: u32,
    shared: Arc<Shared>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        // The thread notices within POLL_MS, stops the stream and releases the client.
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// The capture for the process whose bars are moving, if any.
#[derive(Default)]
pub struct Visualizer {
    capture: Option<Capture>,
}

impl Visualizer {
    /// Captures `pid` (starting or switching as needed), or nothing.
    pub fn follow(&mut self, pid: Option<u32>) {
        if self.capture.as_ref().map(|c| c.pid) == pid {
            return;
        }
        self.capture = None;
        let Some(pid) = pid else { return };
        let shared = Arc::new(Shared::default());
        let thread = Arc::clone(&shared);
        let spawned = std::thread::Builder::new().name("audio-capture".into()).spawn(move || {
            if let Err(e) = capture(pid, &thread) {
                eprintln!("island: audio capture of {pid} unavailable: {e}");
            }
        });
        if spawned.is_ok() {
            self.capture = Some(Capture { pid, shared });
        }
    }

    /// The process being captured.
    pub fn pid(&self) -> Option<u32> {
        self.capture.as_ref().map(|c| c.pid)
    }

    /// Real band levels, or `None` while there's nothing real to show (synthetic bars then).
    pub fn levels(&self) -> Option<[f32; BANDS]> {
        let capture = self.capture.as_ref()?;
        let latest = capture.shared.latest.lock().ok()?;
        let fresh = latest.at.is_some_and(|t| t.elapsed() < SILENCE_LIMIT);
        (latest.audible && fresh).then_some(latest.levels)
    }
}

/// An event handle shared by the capture thread and the activation callback, closed when
/// both are done with it (a late callback never signals a recycled handle).
struct Event(HANDLE);

unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    fn new() -> Result<Arc<Self>> {
        Ok(Arc::new(Event(unsafe { CreateEventW(None, false, false, None)? })))
    }

    fn wait(&self, ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.0, ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { _ = CloseHandle(self.0) };
    }
}

#[windows_core::implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(Arc<Event>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(&self, _: Ref<IActivateAudioInterfaceAsyncOperation>) -> Result<()> {
        unsafe { SetEvent(self.0 .0) }
    }
}

struct Com;

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// Opens a loopback stream of `pid`'s process tree and analyses it until told to stop.
fn capture(pid: u32, shared: &Shared) -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let _com = Com;
    let client = activate(pid)?;
    let format = WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: CHANNELS as u16,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * (CHANNELS * 4) as u32,
        nBlockAlign: (CHANNELS * 4) as u16,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let ready = Event::new()?;
    let reader: IAudioCaptureClient = unsafe {
        // Process loopback has no mix format of its own: it converts to the one given.
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 200_000, 0, &format, None)?;
        client.SetEventHandle(ready.0)?;
        let reader = client.GetService()?;
        client.Start()?;
        reader
    };
    let mut analyzer = Analyzer::default();
    let mut mono = Vec::with_capacity(4096);
    while !shared.stop.load(Ordering::Relaxed) {
        ready.wait(POLL_MS);
        unsafe {
            while reader.GetNextPacketSize()? > 0 {
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                reader.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                mono.clear();
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    mono.resize(frames as usize, 0.0);
                } else {
                    let samples = std::slice::from_raw_parts(data as *const f32, frames as usize * CHANNELS);
                    mono.extend(samples.chunks_exact(CHANNELS).map(|f| f.iter().sum::<f32>() / CHANNELS as f32));
                }
                reader.ReleaseBuffer(frames)?;
                analyzer.push(&mono);
            }
        }
        if let Ok(mut latest) = shared.latest.lock() {
            // `at` only moves when audio arrives, so a stream that goes quiet without
            // packets also falls back after the silence limit.
            if !mono.is_empty() {
                latest.at = Some(Instant::now());
            }
            latest.levels = analyzer.levels;
            latest.audible = analyzer.audible();
        }
        mono.clear();
    }
    unsafe { _ = client.Stop() };
    Ok(())
}

/// `IAudioClient` for loopback of `pid` and its children (browsers play from a child).
fn activate(pid: u32) -> Result<IAudioClient> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS { TargetProcessId: pid, ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE },
        },
    };
    // A blob pointing at `params`, which outlives the call (it copies them). Never dropped:
    // the `windows` crate's `Drop` for PROPVARIANT calls PropVariantClear, which would hand
    // `params` (on this stack) to the COM allocator to free and corrupt the heap.
    let blob = std::mem::ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 { blob: BLOB { cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &params as *const _ as *mut u8 } },
                ..Default::default()
            }),
        },
    });
    let done = Event::new()?;
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(Arc::clone(&done)).into();
    unsafe {
        let operation = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*blob), &handler)?;
        if !done.wait(ACTIVATE_TIMEOUT_MS) {
            return Err(windows::core::Error::from_hresult(HRESULT::from_win32(1460))); // ERROR_TIMEOUT
        }
        let (mut result, mut client) = (HRESULT(0), None);
        operation.GetActivateResult(&mut result, &mut client)?;
        result.ok()?;
        client.ok_or_else(windows::core::Error::empty)?.cast()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Half a second of a sine at `hz`, amplitude `amp`, through a fresh analyzer.
    fn levels_for(hz: f32, amp: f32) -> [f32; BANDS] {
        let mut analyzer = Analyzer::default();
        let samples: Vec<f32> = (0..RATE / 2).map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin()).collect();
        analyzer.push(&samples);
        assert!(analyzer.audible());
        analyzer.levels
    }

    fn loudest(levels: [f32; BANDS]) -> usize {
        (0..BANDS).max_by(|&a, &b| levels[a].total_cmp(&levels[b])).unwrap()
    }

    #[test]
    fn a_tone_lights_its_own_band() {
        for (hz, band) in [(100.0, 0), (500.0, 1), (2_000.0, 2), (8_000.0, 3)] {
            let levels = levels_for(hz, 0.5);
            assert_eq!(loudest(levels), band, "{hz} Hz: {levels:?}");
            assert!(levels[band] > 0.9, "{hz} Hz: {levels:?}");
            for (b, level) in levels.iter().enumerate().filter(|(b, _)| *b != band) {
                assert!(*level < 0.5, "{hz} Hz leaks into band {b}: {levels:?}");
            }
        }
    }

    #[test]
    fn quieter_is_lower() {
        assert!(levels_for(100.0, 0.02)[0] < levels_for(100.0, 0.5)[0]);
        assert!(levels_for(100.0, 0.0001)[0] < 0.1);
    }

    #[test]
    fn bars_rise_fast_and_fall_slowly() {
        let mut analyzer = Analyzer::default();
        let tone: Vec<f32> = (0..HOP * 3).map(|i| 0.5 * (std::f32::consts::TAU * 100.0 * i as f32 / RATE as f32).sin()).collect();
        analyzer.push(&tone);
        // Three hops (50 ms) in, most of the way up.
        assert!(analyzer.levels[0] > 0.8, "{:?}", analyzer.levels);
        let peak = analyzer.levels[0];
        analyzer.push(&vec![0.0; HOP * 3]);
        // The same time later, still well lit.
        assert!(analyzer.levels[0] > peak * 0.4 && analyzer.levels[0] < peak, "{:?}", analyzer.levels);
    }

    #[test]
    fn digital_silence_hands_back_after_the_limit() {
        let mut analyzer = Analyzer::default();
        analyzer.push(&vec![0.0; RATE as usize]);
        assert!(!analyzer.audible(), "never heard anything");
        analyzer.push(&[0.3]);
        assert!(analyzer.audible());
        analyzer.push(&vec![0.0; RATE as usize * 2 + 10]);
        assert!(!analyzer.audible());
    }
}

#[cfg(test)]
mod live {
    /// Captures `REUI_PID` for a few seconds and prints the levels. Manual: needs a playing
    /// app. `REUI_PID=1234 cargo test -p reui-island live_capture -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_capture() {
        let pid: u32 = std::env::var("REUI_PID").unwrap().parse().unwrap();
        let mut v = super::Visualizer::default();
        v.follow(Some(pid));
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            println!("{:?}", v.levels());
        }
        v.follow(None);
        std::thread::sleep(std::time::Duration::from_millis(300));
        println!("stopped cleanly");
    }
}

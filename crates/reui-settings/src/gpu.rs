//! The GPU side of drawing: a D3D11 device, a Direct2D device context on it, and a DXGI
//! flip-model swap chain for the window with a frame-latency waitable object.
//!
//! Why: the old `ID2D1HwndRenderTarget` presented through the window's redirection surface
//! and animations ran on `SetTimer`, which Windows rounds up to its 15.6 ms tick (a 16 ms
//! timer fired every 31 ms). The waitable object tells the message loop when the display
//! can take the next frame, so animations step once per refresh (199 Hz on a 199 Hz
//! panel), and `Present` never blocks the UI thread: a frame is only drawn when the
//! swap chain has room for it. At rest nothing waits on it and nothing is drawn.

use windows::core::{Interface, Result};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

pub const D2DERR_RECREATE_TARGET: i32 = 0x8899_000C_u32 as i32;
const BUFFERS: u32 = 2;
const FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;

pub struct Gpu {
    d3d: ID3D11Device,
    pub factory: ID2D1Factory1,
    pub dc: ID2D1DeviceContext5,
    swap: Option<Swap>,
}

struct Swap {
    chain: IDXGISwapChain2,
    waitable: HANDLE,
    target: Option<ID2D1Bitmap1>,
}

impl Drop for Swap {
    fn drop(&mut self) {
        unsafe { _ = CloseHandle(self.waitable) };
    }
}

fn swap_flags() -> u32 {
    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32
}

impl Gpu {
    pub fn new() -> Result<Self> {
        let d3d = device(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| device(D3D_DRIVER_TYPE_WARP))?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let options = D2D1_FACTORY_OPTIONS { debugLevel: D2D1_DEBUG_LEVEL_NONE };
        let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&options))? };
        let d2d = unsafe { factory.CreateDevice(&dxgi)? };
        let dc: ID2D1DeviceContext5 = unsafe { d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? }.cast()?;
        Ok(Self { d3d, factory, dc, swap: None })
    }

    /// Makes the window's swap chain (once), `width` × `height` pixels.
    pub fn attach(&mut self, hwnd: HWND, width: u32, height: u32) -> Result<()> {
        if self.swap.is_some() {
            return Ok(());
        }
        unsafe {
            let dxgi: IDXGIDevice = self.d3d.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1),
                Height: height.max(1),
                Format: FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: BUFFERS,
                // Unscaled: while a resize is in flight the old frame stays put rather than
                // stretching.
                Scaling: DXGI_SCALING_NONE,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: swap_flags(),
                ..Default::default()
            };
            let chain: IDXGISwapChain2 = factory.CreateSwapChainForHwnd(&self.d3d, hwnd, &desc, None, None)?.cast()?;
            _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            chain.SetMaximumFrameLatency(1)?;
            let waitable = chain.GetFrameLatencyWaitableObject();
            self.swap = Some(Swap { chain, waitable, target: None });
        }
        Ok(())
    }

    /// Signalled when the swap chain can take another frame.
    pub fn waitable(&self) -> Option<HANDLE> {
        self.swap.as_ref().map(|s| s.waitable)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let Some(swap) = self.swap.as_mut() else { return Ok(()) };
        unsafe {
            self.dc.SetTarget(None);
            swap.target = None;
            swap.chain.ResizeBuffers(BUFFERS, width.max(1), height.max(1), DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(swap_flags() as i32))?;
        }
        Ok(())
    }

    /// Points the device context at the back buffer, at `dpi`.
    pub fn begin(&mut self, dpi: f32) -> Result<()> {
        let Some(swap) = self.swap.as_mut() else { return Err(windows::core::Error::from_hresult(windows::core::HRESULT(D2DERR_RECREATE_TARGET))) };
        if swap.target.is_none() {
            unsafe {
                let surface: IDXGISurface = swap.chain.GetBuffer(0)?;
                let props = bitmap_props(dpi, D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW);
                swap.target = Some(self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?);
            }
        }
        let image: Option<ID2D1Image> = swap.target.as_ref().map(|t| t.cast()).transpose()?;
        unsafe {
            self.dc.SetTarget(image.as_ref());
            self.dc.SetDpi(dpi, dpi);
        }
        Ok(())
    }

    /// Shows the frame on the next refresh. Doesn't wait: the message loop only draws when
    /// the waitable object says there's room.
    pub fn present(&self) -> Result<()> {
        let Some(swap) = &self.swap else { return Ok(()) };
        unsafe { swap.chain.Present(1, DXGI_PRESENT(0)).ok() }
    }

    /// An offscreen target `width` × `height` pixels at `dpi`, for screenshots.
    pub fn offscreen(&self, width: u32, height: u32, dpi: f32) -> Result<ID2D1Bitmap1> {
        let props = bitmap_props(dpi, D2D1_BITMAP_OPTIONS_TARGET);
        unsafe { self.dc.CreateBitmap(D2D_SIZE_U { width, height }, None, 0, &props) }
    }

    /// The pixels of `bitmap` (BGRA, rows of `width * 4` bytes).
    pub fn read(&self, bitmap: &ID2D1Bitmap1, width: u32, height: u32) -> Result<Vec<u8>> {
        unsafe {
            let props = bitmap_props(96.0, D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW);
            let cpu = self.dc.CreateBitmap(D2D_SIZE_U { width, height }, None, 0, &props)?;
            cpu.CopyFromBitmap(None, bitmap, None)?;
            let map = cpu.Map(D2D1_MAP_OPTIONS_READ)?;
            let mut out = Vec::with_capacity((width * height * 4) as usize);
            for row in 0..height {
                let line = std::slice::from_raw_parts(map.bits.add((row * map.pitch) as usize), (width * 4) as usize);
                out.extend_from_slice(line);
            }
            cpu.Unmap()?;
            Ok(out)
        }
    }
}

fn bitmap_props(dpi: f32, options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: FORMAT, alphaMode: D2D1_ALPHA_MODE_IGNORE },
        dpiX: dpi,
        dpiY: dpi,
        bitmapOptions: options,
        colorContext: std::mem::ManuallyDrop::new(None),
    }
}

fn device(kind: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(None, kind, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_SINGLETHREADED, Some(&[D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0]), D3D11_SDK_VERSION, Some(&mut device), None, None)?;
    }
    device.ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(D2DERR_RECREATE_TARGET)))
}

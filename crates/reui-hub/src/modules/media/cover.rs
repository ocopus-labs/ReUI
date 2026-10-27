//! Cover art off the player: its thumbnail stream read under a size cap and a timeout,
//! then scaled, trimmed and coloured by `artwork.rs` on a blocking thread.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use reui_protocol::Rgb;
use windows::core::{AgileReference, Interface, Result};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows_future::IAsyncOperation;

use super::artwork;

/// Covers bigger than this are refused; everything is scaled to `ARTWORK_PX` before use.
const MAX_ARTWORK: u64 = 16 * 1024 * 1024;
const ARTWORK_PX: u32 = 512;
/// A player that hangs while handing over its cover or metadata must not hold anything up.
const ARTWORK_TIMEOUT: Duration = Duration::from_secs(2);

/// A cover read and prepared off the hub thread.
pub(super) struct Cover {
    pub(super) png: Vec<u8>,
    pub(super) hash: u64,
    pub(super) accent: Option<Rgb>,
}

/// Reads, scales, trims and colours a cover on a blocking thread, giving up after
/// `ARTWORK_TIMEOUT`. `None` for no cover, a broken one, or a hung player.
pub(super) async fn load_cover(reference: AgileReference<IRandomAccessStreamReference>) -> Option<Cover> {
    let task = tokio::task::spawn_blocking(move || {
        let bytes = read_stream(&reference.resolve().ok()?).ok()?;
        // Scaled once here: smaller to send, hash, decode and draw, and a broken file
        // fails now (music glyph) instead of as a grey tile on the island.
        let artwork::Prepared { png, accent } = artwork::prepare(&bytes, ARTWORK_PX)?;
        let mut hasher = DefaultHasher::new();
        png.hash(&mut hasher);
        Some(Cover { hash: hasher.finish(), accent, png })
    });
    match tokio::time::timeout(ARTWORK_TIMEOUT, task).await {
        Ok(cover) => cover.ok().flatten(),
        Err(_) => {
            eprintln!("media: the player took too long to hand over its artwork");
            None
        }
    }
}

/// Reads a thumbnail stream to its end (its size is sometimes reported as 0), refusing
/// anything over `MAX_ARTWORK`.
fn read_stream(reference: &IRandomAccessStreamReference) -> Result<Vec<u8>> {
    let too_big = || windows::core::Error::new(windows::Win32::Foundation::E_OUTOFMEMORY, "artwork too large");
    let stream = reference.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if size > MAX_ARTWORK {
        return Err(too_big());
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let chunk = if size > 0 { size as u32 } else { 256 * 1024 };
    let mut bytes = Vec::new();
    loop {
        let loaded = reader.LoadAsync(chunk)?.cast::<IAsyncOperation<u32>>()?.join()?;
        if loaded == 0 {
            break;
        }
        let at = bytes.len();
        bytes.resize(at + loaded as usize, 0);
        reader.ReadBytes(&mut bytes[at..])?;
        if bytes.len() as u64 > MAX_ARTWORK {
            return Err(too_big());
        }
        if size > 0 && bytes.len() as u64 >= size {
            break;
        }
    }
    Ok(bytes)
}

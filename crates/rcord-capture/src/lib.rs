//! Screen capture for Go Live.
//!
//! On a Wayland session capture is native: the ScreenCast desktop portal shows the
//! compositor's own picker for a screen or window, and frames arrive from the PipeWire stream
//! it returns. Nothing goes through XWayland. X11 sessions and Windows list monitors directly
//! and capture them with xcap (XShm on X11, DXGI desktop duplication on Windows).
//!
//! Frames come out as tightly packed 32-bit pixels. At most one waits for the consumer:
//! when it falls behind, frames are skipped rather than queued, so latency stays flat.

use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(any(target_os = "linux", windows))]
mod xcap_backend;

/// Byte order of a frame's pixels. The fourth byte is unused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra,
    Rgba,
}

/// One captured picture, rows packed with no padding.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// Something that can be shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceId {
    /// A monitor, by its platform ID.
    Monitor(u32),
    /// Whatever the user picks in the desktop's own screen sharing dialog.
    Portal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: SourceId,
    pub name: String,
    /// Resolution and the like, for the picker.
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("screen sharing was cancelled")]
    Cancelled,
    #[error("that screen is no longer available")]
    NotFound,
    #[error("screen capture is not supported on this system")]
    Unsupported,
    #[error("screen capture failed: {0}")]
    Failed(String),
}

/// True on a native Wayland session, where capture goes through the desktop portal.
pub fn is_wayland() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland")
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// What can be shared. On Wayland this is one entry that opens the system picker, since
/// applications may not look at other windows or screens themselves.
pub fn sources() -> Vec<Source> {
    if is_wayland() {
        return vec![Source {
            id: SourceId::Portal,
            name: "Screen or window".into(),
            detail: "Choose in your desktop's sharing dialog".into(),
        }];
    }
    #[cfg(any(target_os = "linux", windows))]
    {
        xcap_backend::monitors()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

/// A running capture. Dropping it stops capturing and, on Wayland, ends the portal session
/// so the desktop's sharing indicator goes away.
pub struct Capture {
    frames: Receiver<Frame>,
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture").finish_non_exhaustive()
    }
}

impl Capture {
    /// Starts capturing `source` at up to `max_fps` frames a second, with the pointer drawn
    /// in when `cursor` is set and the desktop supports it. On Wayland this blocks while the
    /// user answers the portal dialog, so call it off the UI thread.
    pub fn start(source: &SourceId, max_fps: u32, cursor: bool) -> Result<Self, CaptureError> {
        let max_fps = max_fps.clamp(1, 60);
        // Room for one frame, so a slow encoder skips frames instead of falling behind.
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let stop: Box<dyn FnOnce() + Send> = match source {
            #[cfg(target_os = "linux")]
            SourceId::Portal => Box::new(wayland::start(max_fps, cursor, tx)?),
            #[cfg(any(target_os = "linux", windows))]
            SourceId::Monitor(id) => {
                // Screen capture through xcap leaves the pointer out either way.
                let _ = cursor;
                Box::new(xcap_backend::start(*id, max_fps, tx)?)
            }
            #[allow(unreachable_patterns)]
            _ => return Err(CaptureError::Unsupported),
        };
        Ok(Self {
            frames: rx,
            stop: Some(stop),
        })
    }

    /// Captured frames. The channel closes when capture ends, for example when the user
    /// stops sharing from the desktop's own indicator.
    pub fn frames(&self) -> &Receiver<Frame> {
        &self.frames
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

/// Offers a frame without waiting. Returns false once the receiver is gone.
pub(crate) fn offer(tx: &SyncSender<Frame>, frame: Frame) -> bool {
    !matches!(tx.try_send(frame), Err(TrySendError::Disconnected(_)))
}

/// Copies rows of `stride` bytes into a tightly packed buffer.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn pack_rows(src: &[u8], width: u32, height: u32, stride: usize) -> Option<Vec<u8>> {
    let row = width as usize * 4;
    if stride < row || src.len() < stride * (height as usize - 1) + row {
        return None;
    }
    if stride == row {
        return Some(src[..row * height as usize].to_vec());
    }
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&src[y * stride..y * stride + row]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_padded_rows() {
        let src = [1, 2, 3, 4, 9, 9, 5, 6, 7, 8, 9, 9];
        assert_eq!(pack_rows(&src, 1, 2, 6), Some(vec![1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(pack_rows(&src[..8], 1, 2, 6), None);
    }

    #[test]
    fn frames_are_skipped_not_queued() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let f = |w| Frame {
            width: w,
            height: 1,
            format: PixelFormat::Bgra,
            data: vec![0; 4],
        };
        assert!(offer(&tx, f(1)));
        assert!(offer(&tx, f(2))); // Full: dropped, not blocked.
        assert_eq!(rx.recv().unwrap().width, 1);
        drop(rx);
        assert!(!offer(&tx, f(3)));
    }
}

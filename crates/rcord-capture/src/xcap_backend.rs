//! Monitor capture with xcap: XShm on X11, DXGI desktop duplication on Windows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use xcap::Monitor;

use crate::{CaptureError, Frame, PixelFormat, Source, SourceId};

pub(crate) fn monitors() -> Vec<Source> {
    let Ok(all) = Monitor::all() else {
        return Vec::new();
    };
    let mut out: Vec<(bool, Source)> = all
        .iter()
        .filter_map(|m| {
            let id = m.id().ok()?;
            let name = m
                .friendly_name()
                .ok()
                .filter(|n| !n.trim().is_empty())
                .or_else(|| m.name().ok())
                .unwrap_or_else(|| format!("Screen {id}"));
            let (w, h) = (m.width().unwrap_or(0), m.height().unwrap_or(0));
            let primary = m.is_primary().unwrap_or(false);
            let detail = if primary {
                format!("{w} × {h} · Primary")
            } else {
                format!("{w} × {h}")
            };
            Some((
                primary,
                Source {
                    id: SourceId::Monitor(id),
                    name,
                    detail,
                },
            ))
        })
        .collect();
    out.sort_by_key(|(primary, _)| !primary);
    out.into_iter().map(|(_, s)| s).collect()
}

pub(crate) fn start(
    id: u32,
    max_fps: u32,
    tx: SyncSender<Frame>,
) -> Result<impl FnOnce() + Send + use<>, CaptureError> {
    let monitor = Monitor::all()
        .map_err(|e| CaptureError::Failed(e.to_string()))?
        .into_iter()
        .find(|m| m.id().is_ok_and(|m| m == id))
        .ok_or(CaptureError::NotFound)?;
    let (recorder, frames) = monitor
        .video_recorder()
        .map_err(|e| CaptureError::Failed(e.to_string()))?;
    recorder
        .start()
        .map_err(|e| CaptureError::Failed(e.to_string()))?;

    let running = Arc::new(AtomicBool::new(true));
    let alive = running.clone();
    let interval = Duration::from_secs(1) / max_fps;
    std::thread::Builder::new()
        .name("rcord-capture".into())
        .spawn(move || {
            let mut last: Option<Instant> = None;
            while alive.load(Ordering::Relaxed) {
                let frame = match frames.recv_timeout(Duration::from_millis(250)) {
                    Ok(f) => f,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                // The recorder delivers as fast as it can; keep to the frame rate.
                let now = Instant::now();
                if last.is_some_and(|t| now.duration_since(t) < interval) {
                    continue;
                }
                last = Some(now);
                let expected = frame.width as usize * frame.height as usize * 4;
                if frame.width == 0 || frame.raw.len() < expected {
                    continue;
                }
                let frame = Frame {
                    width: frame.width,
                    height: frame.height,
                    format: PixelFormat::Rgba,
                    data: frame.raw,
                };
                if !crate::offer(&tx, frame) {
                    break;
                }
            }
            let _ = recorder.stop();
        })
        .map_err(|e| CaptureError::Failed(e.to_string()))?;

    Ok(move || running.store(false, Ordering::Relaxed))
}

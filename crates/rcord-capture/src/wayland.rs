//! Native Wayland capture: the ScreenCast desktop portal and its PipeWire stream.
//!
//! The portal conversation runs on its own thread with a small Tokio runtime. Once the user
//! has picked a screen or window, the portal hands over a PipeWire remote and the node to
//! read; a second thread runs the PipeWire loop and copies each buffer out. The portal
//! session stays open until capture stops, which also clears the desktop's sharing
//! indicator.

use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::sync::mpsc::{SyncSender, channel};

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use pipewire as pw;
use pw::spa;
use spa::param::ParamType;
use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::param::video::{VideoFormat, VideoInfoRaw};
use spa::pod::serialize::PodSerializer;
use spa::pod::{self, Pod};
use spa::utils::{Direction, Fraction, Rectangle, SpaTypes};

use crate::{CaptureError, Frame, PixelFormat};

pub(crate) fn start(
    max_fps: u32,
    tx: SyncSender<Frame>,
) -> Result<impl FnOnce() + Send + use<>, CaptureError> {
    let (ready_tx, ready_rx) = channel::<Result<(OwnedFd, u32), CaptureError>>();
    let (close_tx, close_rx) = tokio::sync::oneshot::channel::<()>();

    std::thread::Builder::new()
        .name("rcord-portal".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(CaptureError::Failed(e.to_string())));
                    return;
                }
            };
            rt.block_on(async move {
                let proxy = match Screencast::new().await {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready_tx.send(Err(portal_error(e)));
                        return;
                    }
                };
                let session = match proxy.create_session(Default::default()).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(portal_error(e)));
                        return;
                    }
                };
                let picked = async {
                    let cursor = match proxy.available_cursor_modes().await {
                        Ok(m) if m.contains(CursorMode::Embedded) => Some(CursorMode::Embedded),
                        _ => None,
                    };
                    proxy
                        .select_sources(
                            &session,
                            SelectSourcesOptions::default()
                                .set_cursor_mode(cursor)
                                .set_sources(SourceType::Monitor | SourceType::Window)
                                .set_multiple(false)
                                .set_persist_mode(PersistMode::DoNot),
                        )
                        .await?;
                    let streams = proxy
                        .start(&session, None, Default::default())
                        .await?
                        .response()?;
                    let node = streams
                        .streams()
                        .first()
                        .map(|s| s.pipe_wire_node_id())
                        .ok_or_else(|| {
                            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
                        })?;
                    let fd = proxy
                        .open_pipe_wire_remote(&session, Default::default())
                        .await?;
                    Ok::<_, ashpd::Error>((fd, node))
                }
                .await;
                let ok = picked.is_ok();
                let _ = ready_tx.send(picked.map_err(portal_error));
                if ok {
                    // Keep the session until capture stops.
                    let _ = close_rx.await;
                }
                let _ = session.close().await;
            });
        })
        .map_err(|e| CaptureError::Failed(e.to_string()))?;

    let (fd, node) = ready_rx
        .recv()
        .map_err(|_| CaptureError::Failed("the desktop portal stopped answering".into()))??;

    let (quit_tx, quit_rx) = pw::channel::channel::<()>();
    std::thread::Builder::new()
        .name("rcord-pipewire".into())
        .spawn(move || {
            if let Err(e) = run_stream(fd, node, max_fps, tx, quit_rx) {
                tracing::warn!("PipeWire capture ended: {e}");
            }
        })
        .map_err(|e| CaptureError::Failed(e.to_string()))?;

    Ok(move || {
        let _ = quit_tx.send(());
        let _ = close_tx.send(());
    })
}

fn portal_error(e: ashpd::Error) -> CaptureError {
    match e {
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => CaptureError::Cancelled,
        ashpd::Error::Portal(ashpd::PortalError::Cancelled(_)) => CaptureError::Cancelled,
        other => CaptureError::Failed(format!("desktop portal: {other}")),
    }
}

struct StreamData {
    format: VideoInfoRaw,
    tx: SyncSender<Frame>,
}

fn run_stream(
    fd: OwnedFd,
    node: u32,
    max_fps: u32,
    tx: SyncSender<Frame>,
    quit: pw::channel::Receiver<()>,
) -> Result<(), pw::Error> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_fd_rc(fd, None)?;

    let stream = pw::stream::StreamRc::new(
        core.clone(),
        "rcord screen share",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let data = StreamData {
        format: VideoInfoRaw::default(),
        tx,
    };
    let quit_loop = main_loop.clone();
    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed({
            let main_loop = main_loop.clone();
            move |_, _, _, new| {
                if matches!(new, pw::stream::StreamState::Error(_)) {
                    main_loop.quit();
                }
            }
        })
        .param_changed(|_, data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            if let Err(e) = data.format.parse(param) {
                tracing::warn!("unreadable PipeWire video format: {e:?}");
            }
        })
        .process(|stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(plane) = datas.first_mut() else {
                return;
            };
            let size = data.format.size();
            let (width, height) = (size.width, size.height);
            if width == 0 || height == 0 {
                return;
            }
            let chunk = plane.chunk();
            let offset = chunk.offset() as usize;
            let stride = match chunk.stride() {
                s if s > 0 => s as usize,
                _ => width as usize * 4,
            };
            let format = match data.format.format() {
                VideoFormat::BGRx | VideoFormat::BGRA => PixelFormat::Bgra,
                VideoFormat::RGBx | VideoFormat::RGBA => PixelFormat::Rgba,
                _ => return,
            };
            let Some(bytes) = plane.data() else {
                return; // DMA-BUF without a mapping; only memory buffers are asked for.
            };
            let Some(pixels) = bytes
                .get(offset..)
                .and_then(|b| crate::pack_rows(b, width, height, stride))
            else {
                return;
            };
            let _ = crate::offer(
                &data.tx,
                Frame {
                    width,
                    height,
                    format,
                    data: pixels,
                },
            );
        })
        .register()?;

    let format = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 1920,
                height: 1080
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction {
                num: max_fps,
                denom: 1
            },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );
    let bytes = PodSerializer::serialize(Cursor::new(Vec::new()), &pod::Value::Object(format))
        .map_err(|_| pw::Error::CreationFailed)?
        .0
        .into_inner();
    let mut params = [Pod::from_bytes(&bytes).ok_or(pw::Error::CreationFailed)?];
    stream.connect(
        Direction::Input,
        Some(node),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    let _quit = quit.attach(main_loop.loop_(), move |()| quit_loop.quit());
    main_loop.run();
    let _ = stream.disconnect();
    Ok(())
}

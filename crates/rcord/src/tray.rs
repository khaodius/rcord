//! The notification-area icon that keeps rcord reachable after its window closes, when
//! "Close to tray" is on. Linux uses the StatusNotifierItem protocol through D-Bus (KDE,
//! most panels, and GNOME with the AppIndicator extension); Windows uses the shell's
//! notification area.

use gpui::Global;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Show,
    Quit,
}

/// The running icon. Dropping it removes the icon.
pub struct Tray {
    _inner: imp::Handle,
}

impl Global for Tray {}

const ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/rcord-64.png");

/// The icon as width, height and RGBA bytes.
fn icon_rgba() -> Option<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (w, h) = image.dimensions();
    Some((w, h, image.into_raw()))
}

/// Shows the icon. Events arrive on `events`. `None` when the desktop has no tray to
/// show it in, in which case closing the window should quit as usual.
pub fn start(events: async_channel::Sender<TrayEvent>) -> Option<Tray> {
    match imp::start(events) {
        Ok(inner) => Some(Tray { _inner: inner }),
        Err(e) => {
            tracing::warn!("no tray icon: {e}");
            None
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use ksni::blocking::TrayMethods;

    use super::TrayEvent;

    pub struct Handle(ksni::blocking::Handle<RcordTray>);

    impl Drop for Handle {
        fn drop(&mut self) {
            self.0.shutdown().wait();
        }
    }

    pub struct RcordTray {
        events: async_channel::Sender<TrayEvent>,
        icon: Vec<ksni::Icon>,
    }

    impl ksni::Tray for RcordTray {
        fn id(&self) -> String {
            "rcord".into()
        }

        fn title(&self) -> String {
            "rcord".into()
        }

        fn icon_name(&self) -> String {
            if self.icon.is_empty() {
                "rcord".into()
            } else {
                String::new()
            }
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icon.clone()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.events.try_send(TrayEvent::Show);
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem {
                    label: "Open rcord".into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.events.try_send(TrayEvent::Show);
                    }),
                    ..Default::default()
                }
                .into(),
                ksni::MenuItem::Separator,
                StandardItem {
                    label: "Quit rcord".into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.events.try_send(TrayEvent::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub fn start(events: async_channel::Sender<TrayEvent>) -> anyhow::Result<Handle> {
        let icon = super::icon_rgba()
            .map(|(w, h, mut data)| {
                // RGBA to the ARGB the protocol wants.
                for px in data.as_chunks_mut::<4>().0.iter_mut() {
                    px.rotate_right(1);
                }
                vec![ksni::Icon {
                    width: w as i32,
                    height: h as i32,
                    data,
                }]
            })
            .unwrap_or_default();
        let handle = RcordTray { events, icon }.spawn()?;
        Ok(Handle(handle))
    }
}

#[cfg(windows)]
mod imp {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::TrayEvent;

    pub struct Handle(#[allow(dead_code)] TrayIcon);

    pub fn start(events: async_channel::Sender<TrayEvent>) -> anyhow::Result<Handle> {
        let open = MenuItem::new("Open rcord", true, None);
        let quit = MenuItem::new("Quit rcord", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &PredefinedMenuItem::separator(), &quit])?;
        let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
        let menu_events = events.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == open_id {
                let _ = menu_events.try_send(TrayEvent::Show);
            } else if e.id == quit_id {
                let _ = menu_events.try_send(TrayEvent::Quit);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = events.try_send(TrayEvent::Show);
            }
        }));
        let mut builder = TrayIconBuilder::new()
            .with_tooltip("rcord")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false);
        if let Some((w, h, rgba)) = super::icon_rgba() {
            builder = builder.with_icon(tray_icon::Icon::from_rgba(rgba, w, h)?);
        }
        Ok(Handle(builder.build()?))
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use super::TrayEvent;

    pub struct Handle;

    pub fn start(_: async_channel::Sender<TrayEvent>) -> anyhow::Result<Handle> {
        anyhow::bail!("not supported on this system")
    }
}

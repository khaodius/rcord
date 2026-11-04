//! rcord's icon set: 24x24 stroked outlines, compiled into the binary and served to GPUI's
//! SVG renderer, which draws them in whatever text color the element has.

use std::borrow::Cow;

use gpui::{
    App, AssetSource, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Result, SharedString, StyleRefinement, Styled, Svg, Window, svg,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Logo,
    Hash,
    Announcement,
    Speaker,
    Forum,
    Thread,
    Lock,
    Mic,
    MicOff,
    Headphones,
    HeadphonesOff,
    Gear,
    Plus,
    PlusCircle,
    Gif,
    Sticker,
    Smile,
    Users,
    Search,
    At,
    Pin,
    Reply,
    Trash,
    Copy,
    Close,
    ChevronDown,
    ChevronRight,
    Bell,
    Inbox,
    Edit,
    More,
    Compass,
    Phone,
    PhoneOff,
    Screen,
    Video,
    Check,
    Message,
    UserPlus,
    Gamepad,
    Minimize,
    Maximize,
    Restore,
    Play,
    Pause,
    VolumeOff,
    Volume,
    External,
    Expand,
    ChevronUp,
    Logout,
    Shield,
}

impl Icon {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Logo => "logo",
            Self::Hash => "hash",
            Self::Announcement => "announcement",
            Self::Speaker => "speaker",
            Self::Forum => "forum",
            Self::Thread => "thread",
            Self::Lock => "lock",
            Self::Mic => "mic",
            Self::MicOff => "mic-off",
            Self::Headphones => "headphones",
            Self::HeadphonesOff => "headphones-off",
            Self::Gear => "gear",
            Self::Plus => "plus",
            Self::PlusCircle => "plus-circle",
            Self::Gif => "gif",
            Self::Sticker => "sticker",
            Self::Smile => "smile",
            Self::Users => "users",
            Self::Search => "search",
            Self::At => "at",
            Self::Pin => "pin",
            Self::Reply => "reply",
            Self::Trash => "trash",
            Self::Copy => "copy",
            Self::Close => "close",
            Self::ChevronDown => "chevron-down",
            Self::ChevronUp => "chevron-up",
            Self::Logout => "logout",
            Self::Shield => "shield",
            Self::Message => "message",
            Self::UserPlus => "user-plus",
            Self::Gamepad => "gamepad",
            Self::Minimize => "minimize",
            Self::Maximize => "maximize",
            Self::Restore => "restore",
            Self::Play => "play",
            Self::Pause => "pause",
            Self::VolumeOff => "volume-off",
            Self::Volume => "volume",
            Self::External => "external",
            Self::Expand => "expand",
            Self::ChevronRight => "chevron-right",
            Self::Bell => "bell",
            Self::Inbox => "inbox",
            Self::Edit => "edit",
            Self::More => "more",
            Self::Compass => "compass",
            Self::Phone => "phone",
            Self::PhoneOff => "phone-off",
            Self::Screen => "screen",
            Self::Video => "video",
            Self::Check => "check",
        }
    }

    pub fn path(self) -> SharedString {
        format!("icons/{}.svg", self.name()).into()
    }

    const fn body(self) -> &'static str {
        match self {
            Self::Logo => {
                r#"<path d="M5 4h14a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-7l-4 3v-3H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z"/><path d="M9.5 13.5v-6m0 2.5c.5-1.6 1.8-2.5 3.8-2.5"/><path d="M15 13.2h.01"/>"#
            }
            Self::Hash => r#"<path d="M5 9h15M4 15h15M10 3 8 21M16 3l-2 18"/>"#,
            Self::Announcement => {
                r#"<path d="M3 10v4a1 1 0 0 0 1 1h2l5 4V5L6 9H4a1 1 0 0 0-1 1z"/><path d="M15 9a4 4 0 0 1 0 6M18 6a8 8 0 0 1 0 12"/>"#
            }
            Self::Speaker => {
                r#"<path d="M4 9v6h3l5 4V5L7 9z"/><path d="M16 9a4 4 0 0 1 0 6M19 6a8 8 0 0 1 0 12"/>"#
            }
            Self::Forum => r#"<path d="M4 5h12v9H8l-4 3z"/><path d="M8 17v1h8l4 3V9h-4"/>"#,
            Self::Thread => r#"<path d="M5 4v10a3 3 0 0 0 3 3h11"/><path d="m15 13 4 4-4 4"/>"#,
            Self::Lock => {
                r#"<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>"#
            }
            Self::Mic => {
                r#"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>"#
            }
            Self::MicOff => {
                r#"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3M3 3l18 18"/>"#
            }
            Self::Headphones => {
                r#"<path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="3" y="14" width="5" height="7" rx="2"/><rect x="16" y="14" width="5" height="7" rx="2"/>"#
            }
            Self::HeadphonesOff => {
                r#"<path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="3" y="14" width="5" height="7" rx="2"/><rect x="16" y="14" width="5" height="7" rx="2"/><path d="M3 3l18 18"/>"#
            }
            Self::Gear => {
                r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#
            }
            Self::Plus => r#"<path d="M12 5v14M5 12h14"/>"#,
            Self::PlusCircle => r#"<circle cx="12" cy="12" r="9"/><path d="M12 8v8M8 12h8"/>"#,
            Self::Gif => {
                r#"<rect x="3" y="5" width="18" height="14" rx="3"/><path d="M10 10H8a1 1 0 0 0-1 1v2a1 1 0 0 0 1 1h2v-2M13 10v4M16 14v-4h2M16 12h1.5"/>"#
            }
            Self::Sticker => {
                r#"<path d="M20 12V7a3 3 0 0 0-3-3H7a3 3 0 0 0-3 3v10a3 3 0 0 0 3 3h5z"/><path d="M12 20a8 8 0 0 0 8-8h-5a3 3 0 0 0-3 3z"/>"#
            }
            Self::Smile => {
                r#"<circle cx="12" cy="12" r="9"/><path d="M8 14a5 5 0 0 0 8 0"/><path d="M9 9.5h.01M15 9.5h.01"/>"#
            }
            Self::Users => {
                r#"<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><path d="M16 4.5a3.5 3.5 0 0 1 0 7M18 14a6.5 6.5 0 0 1 3.5 6"/>"#
            }
            Self::Search => r#"<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/>"#,
            Self::At => {
                r#"<circle cx="12" cy="12" r="4"/><path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8"/>"#
            }
            Self::Pin => r#"<path d="M9 4h6l-1 6 4 4H6l4-4z"/><path d="M12 14v7"/>"#,
            Self::Reply => r#"<path d="M10 9V5l-7 7 7 7v-4c5 0 8 1 11 5-1-6-4-11-11-11z"/>"#,
            Self::Trash => r#"<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>"#,
            Self::Copy => {
                r#"<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a2 2 0 0 1 2-2h8"/>"#
            }
            Self::Close => r#"<path d="M6 6l12 12M18 6 6 18"/>"#,
            Self::ChevronDown => r#"<path d="m6 9 6 6 6-6"/>"#,
            Self::ChevronUp => r#"<path d="m6 15 6-6 6 6"/>"#,
            Self::Logout => {
                r#"<path d="M14 4h4a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-4M9 8l-4 4 4 4M5 12h11"/>"#
            }
            Self::Shield => {
                r#"<path d="M12 3 5 6v5c0 4.5 3 8.5 7 10 4-1.5 7-5.5 7-10V6z"/><path d="m9 12 2 2 4-4"/>"#
            }
            Self::ChevronRight => r#"<path d="m9 6 6 6-6 6"/>"#,
            Self::Bell => r#"<path d="M6 16V11a6 6 0 0 1 12 0v5l2 2H4z"/><path d="M10 21h4"/>"#,
            Self::Inbox => r#"<path d="M3 13h5l2 3h4l2-3h5"/><path d="M5 5h14l2 8v6H3v-6z"/>"#,
            Self::Edit => r#"<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="m13 7 4 4"/>"#,
            Self::More => r#"<path d="M5 12h.01M12 12h.01M19 12h.01"/>"#,
            Self::Compass => r#"<circle cx="12" cy="12" r="9"/><path d="m15.5 8.5-2 5-5 2 2-5z"/>"#,
            Self::Phone => {
                r#"<path d="M5 4h4l2 5-2.5 1.5a11 11 0 0 0 5 5L15 13l5 2v4a2 2 0 0 1-2 2A16 16 0 0 1 3 6a2 2 0 0 1 2-2"/>"#
            }
            Self::PhoneOff => {
                r#"<path d="M5 4h4l2 5-2.5 1.5a11 11 0 0 0 5 5L15 13l5 2v4a2 2 0 0 1-2 2A16 16 0 0 1 3 6a2 2 0 0 1 2-2M3 3l18 18"/>"#
            }
            Self::Screen => {
                r#"<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>"#
            }
            Self::Video => {
                r#"<rect x="3" y="6" width="13" height="12" rx="2"/><path d="m16 10 5-3v10l-5-3"/>"#
            }
            Self::Check => r#"<path d="m5 12 5 5 9-10"/>"#,
            Self::Message => r#"<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22z"/>"#,
            Self::UserPlus => {
                r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M19 8v6M22 11h-6"/>"#
            }
            Self::Minimize => r#"<path d="M6 12h12"/>"#,
            Self::Maximize => r#"<rect x="6" y="6" width="12" height="12" rx="1"/>"#,
            Self::Restore => {
                r#"<rect x="5" y="9" width="10" height="10" rx="1"/><path d="M9 9V6a1 1 0 0 1 1-1h8a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1h-3"/>"#
            }
            Self::Play => r#"<path d="M7 4v16l13-8z"/>"#,
            Self::Pause => r#"<path d="M8 5v14M16 5v14"/>"#,
            Self::VolumeOff => r#"<path d="M4 9v6h3l5 4V5L7 9z"/><path d="m16 9 5 6M21 9l-5 6"/>"#,
            Self::Volume => {
                r#"<path d="M4 9v6h3l5 4V5L7 9z"/><path d="M16 9a4 4 0 0 1 0 6M19 6a8 8 0 0 1 0 12"/>"#
            }
            Self::External => {
                r#"<path d="M14 4h6v6M20 4l-9 9"/><path d="M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>"#
            }
            Self::Expand => r#"<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>"#,
            Self::Gamepad => {
                r#"<path d="M6 11h4M8 9v4M15 12h.01M18 10h.01"/><rect x="2" y="6" width="20" height="12" rx="4"/>"#
            }
        }
    }

    pub const ALL: [Self; 52] = [
        Self::Logo,
        Self::Hash,
        Self::Announcement,
        Self::Speaker,
        Self::Forum,
        Self::Thread,
        Self::Lock,
        Self::Mic,
        Self::MicOff,
        Self::Headphones,
        Self::HeadphonesOff,
        Self::Gear,
        Self::Plus,
        Self::PlusCircle,
        Self::Gif,
        Self::Sticker,
        Self::Smile,
        Self::Users,
        Self::Search,
        Self::At,
        Self::Pin,
        Self::Reply,
        Self::Trash,
        Self::Copy,
        Self::Close,
        Self::ChevronDown,
        Self::ChevronRight,
        Self::Bell,
        Self::Inbox,
        Self::Edit,
        Self::More,
        Self::Compass,
        Self::Phone,
        Self::PhoneOff,
        Self::Screen,
        Self::Video,
        Self::Check,
        Self::Message,
        Self::UserPlus,
        Self::Gamepad,
        Self::Minimize,
        Self::Maximize,
        Self::Restore,
        Self::Play,
        Self::Pause,
        Self::VolumeOff,
        Self::Volume,
        Self::External,
        Self::Expand,
        Self::ChevronUp,
        Self::Logout,
        Self::Shield,
    ];

    fn document(self) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }
}

pub fn icon(icon: Icon) -> IconElement {
    IconElement(svg().path(icon.path()).flex_none())
}

/// An SVG icon that takes the surrounding text color unless given its own. GPUI's plain
/// `svg()` draws nothing without an explicit color.
pub struct IconElement(Svg);

impl Styled for IconElement {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl IntoElement for IconElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for IconElement {
    type RequestLayoutState = <Svg as Element>::RequestLayoutState;
    type PrepaintState = <Svg as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.0.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.0.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.0.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.0
            .prepaint(id, inspector_id, bounds, layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let text = self.0.style().text.get_or_insert_default();
        if text.color.is_none() {
            text.color = Some(window.text_style().color);
        }
        self.0
            .paint(id, inspector_id, bounds, layout, prepaint, window, cx);
    }
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let Some(name) = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"))
        else {
            return Ok(None);
        };
        Ok(Icon::ALL
            .iter()
            .find(|i| i.name() == name)
            .map(|i| Cow::Owned(i.document().into_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(if path.is_empty() || path.starts_with("icons") {
            Icon::ALL.iter().map(|i| i.path()).collect()
        } else {
            Vec::new()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_listed_once_and_parses() {
        let mut names: Vec<_> = Icon::ALL.iter().map(|i| i.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Icon::ALL.len());
        for i in Icon::ALL {
            let doc = i.document();
            assert!(doc.starts_with("<svg") && doc.ends_with("</svg>"));
            assert!(Assets.load(&i.path()).unwrap().is_some());
        }
    }
}

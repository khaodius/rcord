//! Lays out parsed message markup. Paragraphs without custom emoji become one wrapped text
//! element with styled ranges, the cheapest form GPUI has; paragraphs with custom emoji are
//! split into text and image pieces that wrap together.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText, IntoElement,
    ObjectFit, Pixels, StrikethroughStyle, StyledText, UnderlineStyle, Window, div, prelude::*, px,
};
use rcord_markup::{Block, Inline, format_timestamp};
use rcord_model::{Snowflake, cdn};
use rcord_store::Store;

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub enum MarkupAction {
    OpenUrl(String),
    User(Snowflake),
    Channel(Snowflake),
    RevealSpoilers,
}

pub type ActionHandler = Rc<dyn Fn(MarkupAction, &mut Window, &mut App)>;

pub struct MarkupCx<'a> {
    pub store: &'a Store,
    pub guild: Option<Snowflake>,
    pub t: &'a Theme,
    pub font_size: Pixels,
    pub animate_emoji: bool,
    pub revealed: bool,
    pub underline_links: bool,
    pub hour24: bool,
    pub utc_offset: time::UtcOffset,
    pub now_secs: i64,
    pub code_font: gpui::SharedString,
    /// Unique per message, for element IDs.
    pub key: u64,
    pub on_action: ActionHandler,
}

#[derive(Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    spoiler: bool,
    code: bool,
}

enum Piece {
    Text(TextPiece),
    Emoji {
        id: Snowflake,
        animated: bool,
        name: String,
    },
}

#[derive(Default)]
struct TextPiece {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    clicks: Vec<(Range<usize>, MarkupAction)>,
}

struct Builder<'a, 'b> {
    cx: &'b MarkupCx<'a>,
    pieces: Vec<Piece>,
    current: TextPiece,
}

impl<'a, 'b> Builder<'a, 'b> {
    fn new(cx: &'b MarkupCx<'a>) -> Self {
        Self {
            cx,
            pieces: Vec::new(),
            current: TextPiece::default(),
        }
    }

    fn style_for(&self, s: Style, extra: HighlightStyle) -> HighlightStyle {
        let t = self.cx.t;
        let mut h = extra;
        if s.bold {
            h.font_weight = Some(FontWeight::BOLD);
        }
        if s.italic {
            h.font_style = Some(FontStyle::Italic);
        }
        if s.underline {
            h.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            });
        }
        if s.strike {
            h.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        if s.code {
            h.background_color = Some(t.code_bg);
        }
        if s.spoiler && !self.cx.revealed {
            h.background_color = Some(t.text_faint);
            h.color = Some(t.text_faint);
        } else if s.spoiler {
            h.background_color = Some(t.code_bg);
        }
        h
    }

    fn push(&mut self, text: &str, s: Style, extra: HighlightStyle, click: Option<MarkupAction>) {
        if text.is_empty() {
            return;
        }
        let start = self.current.text.len();
        self.current.text.push_str(text);
        let range = start..self.current.text.len();
        let h = self.style_for(s, extra);
        if h != HighlightStyle::default() {
            self.current.highlights.push((range.clone(), h));
        }
        let click = if s.spoiler && !self.cx.revealed {
            Some(MarkupAction::RevealSpoilers)
        } else {
            click
        };
        if let Some(action) = click {
            self.current.clicks.push((range, action));
        }
    }

    fn walk(&mut self, inlines: &[Inline], s: Style) {
        let t = self.cx.t;
        for inline in inlines {
            match inline {
                Inline::Text(text) => self.push(text, s, HighlightStyle::default(), None),
                Inline::LineBreak => self.push("\n", s, HighlightStyle::default(), None),
                Inline::Bold(c) => self.walk(c, Style { bold: true, ..s }),
                Inline::Italic(c) => self.walk(c, Style { italic: true, ..s }),
                Inline::Underline(c) => self.walk(
                    c,
                    Style {
                        underline: true,
                        ..s
                    },
                ),
                Inline::Strike(c) => self.walk(c, Style { strike: true, ..s }),
                Inline::Spoiler(c) => self.walk(c, Style { spoiler: true, ..s }),
                Inline::Code(code) => self.push(
                    code,
                    Style { code: true, ..s },
                    HighlightStyle::default(),
                    None,
                ),
                Inline::Link { url, label, .. } => {
                    let link = HighlightStyle {
                        color: Some(t.link),
                        underline: self.cx.underline_links.then_some(UnderlineStyle {
                            thickness: px(1.),
                            color: None,
                            wavy: false,
                        }),
                        ..HighlightStyle::default()
                    };
                    let action = Some(MarkupAction::OpenUrl(url.clone()));
                    match label {
                        Some(label) => {
                            let text = rcord_markup::plain_text(label);
                            self.push(&text, s, link, action);
                        }
                        None => self.push(url, s, link, action),
                    }
                }
                Inline::UserMention(id) => {
                    let Some(id) = Snowflake::new(*id) else {
                        continue;
                    };
                    let name = self
                        .cx
                        .store
                        .display_name(self.cx.guild, id)
                        .map_or_else(|| "@unknown-user".to_owned(), |n| format!("@{n}"));
                    self.push(&name, s, self.mention(), Some(MarkupAction::User(id)));
                }
                Inline::ChannelMention(id) => {
                    let Some(id) = Snowflake::new(*id) else {
                        continue;
                    };
                    let name = if self.cx.store.channel(id).is_some() {
                        format!("#{}", self.cx.store.channel_name(id))
                    } else {
                        "#unknown".into()
                    };
                    self.push(&name, s, self.mention(), Some(MarkupAction::Channel(id)));
                }
                Inline::RoleMention(id) => {
                    let role = Snowflake::new(*id).and_then(|rid| {
                        self.cx
                            .guild
                            .and_then(|g| self.cx.store.guild(g))
                            .and_then(|g| g.guild.roles.iter().find(|r| r.id == rid))
                    });
                    let (name, color) = match role {
                        Some(r) => (format!("@{}", r.name), r.color),
                        None => ("@unknown-role".into(), 0),
                    };
                    let style = if color != 0 {
                        let c: Hsla = gpui::rgb(color).into();
                        HighlightStyle {
                            color: Some(c),
                            background_color: Some(Hsla { a: 0.15, ..c }),
                            ..HighlightStyle::default()
                        }
                    } else {
                        self.mention()
                    };
                    self.push(&name, s, style, None);
                }
                Inline::Everyone => self.push("@everyone", s, self.mention(), None),
                Inline::Here => self.push("@here", s, self.mention(), None),
                Inline::Command { name, .. } => {
                    self.push(&format!("/{name}"), s, self.mention(), None)
                }
                Inline::CustomEmoji { name, id, animated } => {
                    let Some(id) = Snowflake::new(*id) else {
                        continue;
                    };
                    let done = std::mem::take(&mut self.current);
                    if !done.text.is_empty() {
                        self.pieces.push(Piece::Text(done));
                    }
                    self.pieces.push(Piece::Emoji {
                        id,
                        animated: *animated,
                        name: name.clone(),
                    });
                }
                Inline::Timestamp { unix, style } => {
                    let text = format_timestamp(
                        *unix,
                        *style,
                        self.cx.now_secs,
                        self.cx.utc_offset,
                        self.cx.hour24,
                    );
                    let h = HighlightStyle {
                        background_color: Some(t.code_bg),
                        ..HighlightStyle::default()
                    };
                    self.push(&text, s, h, None);
                }
            }
        }
    }

    fn mention(&self) -> HighlightStyle {
        HighlightStyle {
            color: Some(self.cx.t.mention_text),
            background_color: Some(self.cx.t.mention_bg),
            font_weight: Some(FontWeight::MEDIUM),
            ..HighlightStyle::default()
        }
    }

    fn finish(mut self) -> Vec<Piece> {
        let last = std::mem::take(&mut self.current);
        if !last.text.is_empty() {
            self.pieces.push(Piece::Text(last));
        }
        self.pieces
    }
}

fn text_element(piece: TextPiece, id: (u64, usize), on_action: &ActionHandler) -> AnyElement {
    let styled = StyledText::new(piece.text).with_highlights(piece.highlights);
    if piece.clicks.is_empty() {
        return styled.into_any_element();
    }
    let (ranges, actions): (Vec<_>, Vec<_>) = piece.clicks.into_iter().unzip();
    let on_action = on_action.clone();
    InteractiveText::new(
        gpui::ElementId::NamedInteger(format!("md-{}", id.1).into(), id.0),
        styled,
    )
    .on_click(ranges, move |ix, window, cx| {
        if let Some(a) = actions.get(ix) {
            on_action(a.clone(), window, cx);
        }
    })
    .into_any_element()
}

/// Inline content as elements: one text element, or wrapped pieces around custom emoji.
pub fn inline(inlines: &[Inline], cx: &MarkupCx<'_>, jumbo: bool, salt: usize) -> AnyElement {
    let mut b = Builder::new(cx);
    b.walk(inlines, Style::default());
    let pieces = b.finish();
    let emoji_size = if jumbo { px(48.) } else { cx.font_size * 1.375 };
    let only_text = pieces.iter().all(|p| matches!(p, Piece::Text(_)));
    if only_text {
        let mut pieces = pieces;
        return match pieces.pop() {
            Some(Piece::Text(p)) if pieces.is_empty() => {
                let el = text_element(p, (cx.key, salt), &cx.on_action);
                if jumbo {
                    div()
                        .text_size(px(44.))
                        .line_height(px(52.))
                        .child(el)
                        .into_any_element()
                } else {
                    el
                }
            }
            _ => div().into_any_element(),
        };
    }
    let mut row = div().flex().flex_row().flex_wrap().items_center();
    for (i, piece) in pieces.into_iter().enumerate() {
        row = row.child(match piece {
            Piece::Text(p) => {
                let el = text_element(p, (cx.key, salt * 1000 + i), &cx.on_action);
                if jumbo {
                    div().text_size(px(44.)).child(el).into_any_element()
                } else {
                    el
                }
            }
            Piece::Emoji { id, animated, name } => {
                let url = cdn::custom_emoji(
                    id,
                    animated && cx.animate_emoji,
                    (f32::from(emoji_size) * 2.0) as u32,
                );
                crate::ui::anim_img(url)
                    .size(emoji_size)
                    .mx(px(1.))
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || div().child(format!(":{name}:")).into_any_element())
                    .into_any_element()
            }
        });
    }
    row.into_any_element()
}

pub fn blocks(blocks: &[Block], cx: &MarkupCx<'_>) -> AnyElement {
    let jumbo = rcord_markup::is_jumbo(blocks);
    let mut col = div().w_full().flex().flex_col().min_w_0().gap(px(2.));
    for (i, block) in blocks.iter().enumerate() {
        col = col.child(block_element(block, cx, jumbo, i));
    }
    col.into_any_element()
}

fn block_element(block: &Block, cx: &MarkupCx<'_>, jumbo: bool, salt: usize) -> AnyElement {
    let t = cx.t;
    match block {
        Block::Paragraph(inlines) => inline(inlines, cx, jumbo, salt),
        Block::Heading { level, content } => {
            let size = match level {
                1 => cx.font_size * 1.5,
                2 => cx.font_size * 1.25,
                _ => cx.font_size * 1.0,
            };
            div()
                .mt(px(4.))
                .text_size(size)
                .line_height(size * 1.3)
                .font_weight(FontWeight::BOLD)
                .text_color(t.header)
                .child(inline(content, cx, false, salt))
                .into_any_element()
        }
        Block::Subtext(content) => div()
            .text_size(cx.font_size * 0.8)
            .text_color(t.text_muted)
            .child(inline(content, cx, false, salt))
            .into_any_element(),
        Block::Quote(children) => div()
            .flex()
            .flex_row()
            .gap(px(10.))
            .child(div().w(px(4.)).flex_none().rounded(px(2.)).bg(t.text_faint))
            .child(div().flex_1().min_w_0().child(blocks(children, cx)))
            .into_any_element(),
        Block::List {
            ordered,
            start,
            items,
        } => {
            let mut col = div().flex().flex_col().gap(px(2.)).pl(px(4.));
            for (n, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}.", *start as usize + n)
                } else {
                    "•".into()
                };
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_none()
                                .min_w(px(14.))
                                .text_color(t.text_muted)
                                .child(marker),
                        )
                        .child(div().flex_1().min_w_0().child(inline(
                            item,
                            cx,
                            false,
                            salt * 100 + n,
                        ))),
                );
            }
            col.into_any_element()
        }
        Block::Code { language, code } => div()
            .w_full()
            .mt(px(2.))
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .rounded(t.radius_small)
            .bg(t.code_bg)
            .border_1()
            .border_color(t.border)
            .font_family(cx.code_font.clone())
            .text_size(cx.font_size * 0.875)
            .line_height(cx.font_size * 1.25)
            .text_color(t.text)
            .when_some(language.clone(), |el, lang| {
                el.child(div().text_xs().text_color(t.text_faint).child(lang))
            })
            .child(code.clone())
            .into_any_element(),
    }
}

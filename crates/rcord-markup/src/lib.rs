//! Discord's message markup: the Markdown subset, mentions, custom emoji, timestamps and
//! links, parsed once into a tree the UI lays out without re-scanning text.

mod timestamp;

pub use timestamp::{TimestampStyle, format_timestamp};

/// One line-level element of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    /// `-# small text`.
    Subtext(Vec<Inline>),
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: u32,
        items: Vec<Vec<Inline>>,
    },
    Code {
        language: Option<String>,
        code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    LineBreak,
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Underline(Vec<Inline>),
    Strike(Vec<Inline>),
    Spoiler(Vec<Inline>),
    Code(String),
    Link {
        url: String,
        label: Option<Vec<Inline>>,
        embed: bool,
    },
    UserMention(u64),
    ChannelMention(u64),
    RoleMention(u64),
    Everyone,
    Here,
    Command {
        name: String,
        id: u64,
    },
    CustomEmoji {
        name: String,
        id: u64,
        animated: bool,
    },
    Timestamp {
        unix: i64,
        style: TimestampStyle,
    },
}

/// Parses a whole message.
pub fn parse(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines = source.split('\n').peekable();
    let mut paragraph: Vec<&str> = Vec::new();

    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(parse_inline(&paragraph.join("\n"))));
            paragraph.clear();
        }
    };

    while let Some(line) = lines.next() {
        // Fenced code, which may also open and close on one line.
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            if let Some(end) = rest.find("```") {
                flush(&mut paragraph, &mut blocks);
                blocks.push(Block::Code {
                    language: None,
                    code: rest[..end].to_owned(),
                });
                let after = &rest[end + 3..];
                if !after.trim().is_empty() {
                    paragraph.push(after);
                }
                continue;
            }
            let mut body = Vec::new();
            let mut closed = false;
            let mut tail = "";
            for next in lines.by_ref() {
                if let Some(pos) = next.find("```") {
                    body.push(&next[..pos]);
                    tail = &next[pos + 3..];
                    closed = true;
                    break;
                }
                body.push(next);
            }
            if !closed {
                // An unclosed fence is plain text, as in Discord.
                paragraph.push(line);
                paragraph.extend(body);
                continue;
            }
            flush(&mut paragraph, &mut blocks);
            let language = Some(rest.trim())
                .filter(|l| !l.is_empty() && !l.contains(char::is_whitespace))
                .map(str::to_owned);
            let mut code_lines = body;
            if language.is_none() && !rest.trim().is_empty() {
                code_lines.insert(0, rest);
            }
            if code_lines.last().is_some_and(|l| l.is_empty()) {
                code_lines.pop();
            }
            blocks.push(Block::Code {
                language,
                code: code_lines.join("\n"),
            });
            if !tail.trim().is_empty() {
                paragraph.push(tail);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix(">>> ") {
            flush(&mut paragraph, &mut blocks);
            let mut quoted = vec![rest];
            quoted.extend(lines.by_ref());
            blocks.push(Block::Quote(parse(&quoted.join("\n"))));
            break;
        }

        if line.starts_with("> ") || line == ">" {
            flush(&mut paragraph, &mut blocks);
            let mut quoted = vec![line.strip_prefix("> ").unwrap_or("")];
            while let Some(next) = lines.peek() {
                if let Some(q) = next.strip_prefix("> ") {
                    quoted.push(q);
                    lines.next();
                } else if *next == ">" {
                    quoted.push("");
                    lines.next();
                } else {
                    break;
                }
            }
            blocks.push(Block::Quote(parse(&quoted.join("\n"))));
            continue;
        }

        if let Some((level, rest)) = heading(line) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading {
                level,
                content: parse_inline(rest),
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("-# ")
            && !rest.trim().is_empty()
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Subtext(parse_inline(rest)));
            continue;
        }

        if let Some((ordered, start, item)) = list_item(line) {
            flush(&mut paragraph, &mut blocks);
            let mut items = vec![parse_inline(item)];
            while let Some(next) = lines.peek() {
                match list_item(next) {
                    Some((o, _, item)) if o == ordered => {
                        items.push(parse_inline(item));
                        lines.next();
                    }
                    _ => break,
                }
            }
            blocks.push(Block::List {
                ordered,
                start,
                items,
            });
            continue;
        }

        paragraph.push(line);
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

fn heading(line: &str) -> Option<(u8, &str)> {
    for (level, prefix) in [(3u8, "### "), (2, "## "), (1, "# ")] {
        if let Some(rest) = line.strip_prefix(prefix)
            && !rest.trim().is_empty()
        {
            return Some((level, rest));
        }
    }
    None
}

fn list_item(line: &str) -> Option<(bool, u32, &str)> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return Some((false, 1, rest));
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && digits <= 9 {
        let rest = &trimmed[digits..];
        if let Some(item) = rest.strip_prefix(". ") {
            return Some((true, trimmed[..digits].parse().ok()?, item));
        }
    }
    None
}

/// Parses text inside a block.
pub fn parse_inline(source: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut text = String::new();
    let bytes = source.as_bytes();
    let mut i = 0;

    macro_rules! push_text {
        () => {
            if !text.is_empty() {
                out.push(Inline::Text(std::mem::take(&mut text)));
            }
        };
    }

    while i < bytes.len() {
        let rest = &source[i..];
        let prev = source[..i].chars().next_back();

        if bytes[i] == b'\\'
            && let Some(c) = rest[1..].chars().next()
            && c.is_ascii_punctuation()
        {
            text.push(c);
            i += 1 + c.len_utf8();
            continue;
        }

        if bytes[i] == b'\n' {
            push_text!();
            out.push(Inline::LineBreak);
            i += 1;
            continue;
        }

        if bytes[i] == b'`' {
            let ticks = if rest.starts_with("``") { 2 } else { 1 };
            let fence = &rest[..ticks];
            if let Some(end) = rest[ticks..].find(fence)
                && end > 0
            {
                push_text!();
                let code = &rest[ticks..ticks + end];
                let code = if ticks == 2 { code.trim() } else { code };
                out.push(Inline::Code(code.to_owned()));
                i += ticks * 2 + end;
                continue;
            }
        }

        if rest.starts_with("***")
            && let Some(end) = find_closing(&rest[3..], "***")
        {
            push_text!();
            out.push(Inline::Bold(vec![Inline::Italic(parse_inline(
                &rest[3..3 + end],
            ))]));
            i += 6 + end;
            continue;
        }

        if let Some((delim, wrap)) = PAIRED.iter().find(|(d, _)| rest.starts_with(*d))
            && let Some(end) = find_closing(&rest[delim.len()..], delim)
        {
            push_text!();
            let inner = parse_inline(&rest[delim.len()..delim.len() + end]);
            out.push(wrap(inner));
            i += delim.len() * 2 + end;
            continue;
        }

        if bytes[i] == b'*'
            && !rest.starts_with("**")
            && let Some(end) = find_single(&rest[1..], '*')
        {
            push_text!();
            out.push(Inline::Italic(parse_inline(&rest[1..1 + end])));
            i += 2 + end;
            continue;
        }

        if bytes[i] == b'_'
            && !rest.starts_with("__")
            && !prev.is_some_and(char::is_alphanumeric)
            && let Some(end) = find_single(&rest[1..], '_')
            && !rest[2 + end..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
        {
            push_text!();
            out.push(Inline::Italic(parse_inline(&rest[1..1 + end])));
            i += 2 + end;
            continue;
        }

        if bytes[i] == b'<'
            && let Some(end) = rest.find('>')
            && let Some(token) = angle_token(&rest[1..end])
        {
            push_text!();
            out.push(token);
            i += end + 1;
            continue;
        }

        if bytes[i] == b'@' {
            if rest.starts_with("@everyone") {
                push_text!();
                out.push(Inline::Everyone);
                i += "@everyone".len();
                continue;
            }
            if rest.starts_with("@here") {
                push_text!();
                out.push(Inline::Here);
                i += "@here".len();
                continue;
            }
        }

        if bytes[i] == b'['
            && let Some((label, url, len)) = masked_link(rest)
        {
            push_text!();
            out.push(Inline::Link {
                url: url.to_owned(),
                label: Some(parse_inline(label)),
                embed: true,
            });
            i += len;
            continue;
        }

        if (rest.starts_with("https://") || rest.starts_with("http://"))
            && !prev.is_some_and(char::is_alphanumeric)
        {
            let len = url_len(rest);
            if len > "https://".len() {
                push_text!();
                out.push(Inline::Link {
                    url: rest[..len].to_owned(),
                    label: None,
                    embed: true,
                });
                i += len;
                continue;
            }
        }

        let c = rest.chars().next().unwrap_or_default();
        text.push(c);
        i += c.len_utf8();
    }
    push_text!();
    out
}

type Wrap = fn(Vec<Inline>) -> Inline;

const PAIRED: [(&str, Wrap); 4] = [
    ("**", Inline::Bold),
    ("__", Inline::Underline),
    ("~~", Inline::Strike),
    ("||", Inline::Spoiler),
];

/// Finds `delim` closing a span that starts at the beginning of `s`, skipping escapes and
/// code spans. Empty spans do not count.
fn find_closing(s: &str, delim: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            // Byte comparisons throughout: `i` can land inside a multi-byte character after
            // an escape, where slicing the `str` would panic.
            b'`' => match bytes[i + 1..].iter().position(|b| *b == b'`') {
                Some(end) => i += end + 2,
                None => i += 1,
            },
            _ if bytes[i..].starts_with(delim.as_bytes()) && i > 0 => {
                // In `**b *c***` the bold closes on the last two stars, after the italic.
                let mut end = i;
                while bytes[end + 1..].starts_with(delim.as_bytes()) && delim.len() > 1 {
                    end += 1;
                }
                return Some(end);
            }
            _ => i += 1,
        }
    }
    None
}

/// Like [`find_closing`] for a one-character delimiter that must not be doubled and must
/// not follow whitespace.
fn find_single(s: &str, delim: char) -> Option<usize> {
    let doubled = [delim as u8; 2];
    let bytes = s.as_bytes();
    if bytes.first().is_some_and(|b| b.is_ascii_whitespace()) {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == delim as u8 {
            if bytes[i..].starts_with(&doubled) {
                // Skip a nested bold or underline span whole.
                let inner = &s[i + 2..];
                let end = find_closing(inner, std::str::from_utf8(&doubled).unwrap_or_default())?;
                i += end + 4;
                continue;
            }
            if i > 0 && !bytes[i - 1].is_ascii_whitespace() {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn angle_token(inner: &str) -> Option<Inline> {
    let id = |s: &str| s.parse::<u64>().ok().filter(|v| *v != 0);
    if inner.starts_with("https://") || inner.starts_with("http://") {
        return (!inner.contains(char::is_whitespace)).then(|| Inline::Link {
            url: inner.to_owned(),
            label: None,
            embed: false,
        });
    }
    if let Some(rest) = inner.strip_prefix("@&") {
        return id(rest).map(Inline::RoleMention);
    }
    if let Some(rest) = inner.strip_prefix('@') {
        return id(rest.strip_prefix('!').unwrap_or(rest)).map(Inline::UserMention);
    }
    if let Some(rest) = inner.strip_prefix('#') {
        return id(rest).map(Inline::ChannelMention);
    }
    if let Some(rest) = inner.strip_prefix("t:") {
        let (unix, style) = match rest.split_once(':') {
            Some((n, s)) => (n, TimestampStyle::from_char(s.chars().next()?)?),
            None => (rest, TimestampStyle::ShortDateTime),
        };
        return Some(Inline::Timestamp {
            unix: unix.parse().ok()?,
            style,
        });
    }
    if let Some(rest) = inner.strip_prefix('/') {
        let (name, cmd_id) = rest.rsplit_once(':')?;
        return Some(Inline::Command {
            name: name.to_owned(),
            id: id(cmd_id)?,
        });
    }
    let (animated, rest) = match inner.strip_prefix("a:") {
        Some(rest) => (true, rest),
        None => (false, inner.strip_prefix(':')?),
    };
    let (name, emoji_id) = rest.split_once(':')?;
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(Inline::CustomEmoji {
        name: name.to_owned(),
        id: id(emoji_id)?,
        animated,
    })
}

fn masked_link(s: &str) -> Option<(&str, &str, usize)> {
    let close = s.find("](")?;
    let label = &s[1..close];
    if label.is_empty() || label.contains('\n') {
        return None;
    }
    let after = &s[close + 2..];
    let end = after.find(')')?;
    let url = after[..end]
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.contains(' ') {
        return None;
    }
    Some((label, url, close + 2 + end + 1))
}

/// Length of a bare URL, dropping trailing punctuation and unbalanced closing parentheses.
fn url_len(s: &str) -> usize {
    let mut end = s
        .find(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"')
        .unwrap_or(s.len());
    loop {
        let candidate = &s[..end];
        let Some(last) = candidate.chars().next_back() else {
            return 0;
        };
        let opens = candidate.matches('(').count();
        let closes = candidate.matches(')').count();
        if matches!(
            last,
            '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '*' | '_' | '~' | '|'
        ) || (last == ')' && closes > opens)
        {
            end -= last.len_utf8();
        } else {
            return end;
        }
    }
}

/// Whether a message is only emoji and whitespace, which Discord draws large. Unicode emoji
/// arrive as text, so any non-whitespace text that is not an emoji character disqualifies.
pub fn is_jumbo(blocks: &[Block]) -> bool {
    let [Block::Paragraph(inlines)] = blocks else {
        return false;
    };
    let mut count = 0;
    for inline in inlines {
        match inline {
            Inline::CustomEmoji { .. } => count += 1,
            Inline::LineBreak => {}
            Inline::Text(t) => {
                for c in t.chars().filter(|c| !c.is_whitespace()) {
                    if is_emoji_char(c) {
                        count += 1;
                    } else if !is_emoji_modifier(c) {
                        return false;
                    }
                }
            }
            _ => return false,
        }
    }
    count > 0 && count <= 30
}

fn is_emoji_char(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x2190..=0x21FF
        | 0x2300..=0x23FF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x00A9 | 0x00AE | 0x203C
        | 0x2049 | 0x2122 | 0x2139)
}

fn is_emoji_modifier(c: char) -> bool {
    matches!(
        c as u32,
        0x200D | 0xFE0F | 0xFE0E | 0x20E3 | 0xE0020..=0xE007F
    )
}

/// Plain text of inline content, for notifications, reply previews and search.
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    write_plain(inlines, &mut out);
    out
}

fn write_plain(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::LineBreak => out.push(' '),
            Inline::Bold(c)
            | Inline::Italic(c)
            | Inline::Underline(c)
            | Inline::Strike(c)
            | Inline::Spoiler(c) => write_plain(c, out),
            Inline::Link { url, label, .. } => match label {
                Some(l) => write_plain(l, out),
                None => out.push_str(url),
            },
            Inline::UserMention(_) => out.push_str("@user"),
            Inline::ChannelMention(_) => out.push_str("#channel"),
            Inline::RoleMention(_) => out.push_str("@role"),
            Inline::Everyone => out.push_str("@everyone"),
            Inline::Here => out.push_str("@here"),
            Inline::Command { name, .. } => {
                out.push('/');
                out.push_str(name);
            }
            Inline::CustomEmoji { name, .. } => {
                out.push(':');
                out.push_str(name);
                out.push(':');
            }
            Inline::Timestamp { unix, .. } => out.push_str(&unix.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn multibyte_text_after_markers_does_not_panic() {
        parse("```rust\n||-# ```rust\n(\u{1F600}<*\u{1F600}**> (\u{e9}");
    }

    /// Random mixes of markers and multi-byte text, the way a long channel scrolls past
    /// them. Any panic here took the whole client down while rendering a message.
    #[test]
    fn random_markup_never_panics() {
        let atoms = [
            "*",
            "**",
            "_",
            "__",
            "~~",
            "||",
            "`",
            "```",
            "\n",
            "> ",
            ">>> ",
            "# ",
            "-# ",
            "- ",
            "1. ",
            "<@1>",
            "<#2>",
            "<t:5:R>",
            "<a:x:3>",
            "[a](http://x)",
            "\u{e9}",
            "\u{65e5}",
            "\u{1F600}",
            "\\",
            "@everyone",
            "http://e.com",
            "<",
            ">",
            "]",
            "(",
            ")",
            ":",
            "a",
            " ",
            "<@&",
            "<t:",
            "```rust\n",
            "[",
            "\t",
        ];
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..40_000 {
            let mut s = String::new();
            for _ in 0..(seed % 24) as usize + 1 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push_str(atoms[(seed % atoms.len() as u64) as usize]);
            }
            is_jumbo(&parse(&s));
            plain_text(&parse_inline(&s));
        }
    }

    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn emphasis() {
        assert_eq!(
            parse_inline("a **b *c*** d"),
            vec![
                t("a "),
                Inline::Bold(vec![t("b "), Inline::Italic(vec![t("c")])]),
                t(" d"),
            ]
        );
        assert_eq!(
            parse_inline("__u__ ~~s~~ ||sp||"),
            vec![
                Inline::Underline(vec![t("u")]),
                t(" "),
                Inline::Strike(vec![t("s")]),
                t(" "),
                Inline::Spoiler(vec![t("sp")]),
            ]
        );
        assert_eq!(
            parse_inline("***both***"),
            vec![Inline::Bold(vec![Inline::Italic(vec![t("both")])])]
        );
    }

    #[test]
    fn underscores_inside_words_stay_text() {
        assert_eq!(parse_inline("snake_case_name"), vec![t("snake_case_name")]);
        assert_eq!(parse_inline("_it_"), vec![Inline::Italic(vec![t("it")])]);
    }

    #[test]
    fn escapes_and_code() {
        assert_eq!(parse_inline(r"\*not\*"), vec![t("*not*")]);
        assert_eq!(
            parse_inline("`**raw**` ``a`b``"),
            vec![
                Inline::Code("**raw**".into()),
                t(" "),
                Inline::Code("a`b".into())
            ]
        );
    }

    #[test]
    fn tokens() {
        assert_eq!(
            parse_inline("<@!1> <#2> <@&3> <a:wave:4> <t:1700000000:R> </ping:5> @here"),
            vec![
                Inline::UserMention(1),
                t(" "),
                Inline::ChannelMention(2),
                t(" "),
                Inline::RoleMention(3),
                t(" "),
                Inline::CustomEmoji {
                    name: "wave".into(),
                    id: 4,
                    animated: true
                },
                t(" "),
                Inline::Timestamp {
                    unix: 1_700_000_000,
                    style: TimestampStyle::Relative
                },
                t(" "),
                Inline::Command {
                    name: "ping".into(),
                    id: 5
                },
                t(" "),
                Inline::Here,
            ]
        );
        assert_eq!(parse_inline("a < b > c"), vec![t("a < b > c")]);
    }

    #[test]
    fn links() {
        assert_eq!(
            parse_inline("see (https://example.com/a_(b)), ok"),
            vec![
                t("see ("),
                Inline::Link {
                    url: "https://example.com/a_(b)".into(),
                    label: None,
                    embed: true
                },
                t("), ok"),
            ]
        );
        assert_eq!(
            parse_inline("<https://x.dev>"),
            vec![Inline::Link {
                url: "https://x.dev".into(),
                label: None,
                embed: false
            }]
        );
        assert_eq!(
            parse_inline("[docs](https://docs.rs)"),
            vec![Inline::Link {
                url: "https://docs.rs".into(),
                label: Some(vec![t("docs")]),
                embed: true
            }]
        );
    }

    #[test]
    fn blocks() {
        let b = parse(
            "# Title\nline one\nline two\n> quoted\n> more\n- a\n- b\n```rs\nfn x() {}\n```\n-# small",
        );
        assert_eq!(
            b,
            vec![
                Block::Heading {
                    level: 1,
                    content: vec![t("Title")]
                },
                Block::Paragraph(vec![t("line one"), Inline::LineBreak, t("line two")]),
                Block::Quote(vec![Block::Paragraph(vec![
                    t("quoted"),
                    Inline::LineBreak,
                    t("more")
                ])]),
                Block::List {
                    ordered: false,
                    start: 1,
                    items: vec![vec![t("a")], vec![t("b")]]
                },
                Block::Code {
                    language: Some("rs".into()),
                    code: "fn x() {}".into()
                },
                Block::Subtext(vec![t("small")]),
            ]
        );
    }

    #[test]
    fn block_quote_takes_the_rest() {
        assert_eq!(
            parse(">>> a\nb"),
            vec![Block::Quote(vec![Block::Paragraph(vec![
                t("a"),
                Inline::LineBreak,
                t("b")
            ])])]
        );
    }

    #[test]
    fn unclosed_fence_is_text() {
        assert_eq!(parse("```oops"), vec![Block::Paragraph(vec![t("```oops")])]);
    }

    #[test]
    fn jumbo() {
        assert!(is_jumbo(&parse("🔥 <:a:1>")));
        assert!(is_jumbo(&parse("👍🏽")));
        assert!(!is_jumbo(&parse("hi 🔥")));
    }
}

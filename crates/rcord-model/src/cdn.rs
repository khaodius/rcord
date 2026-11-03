//! URLs for images on Discord's CDN. Sizes are rounded up to a power of two between 16 and
//! 4096, the only sizes the CDN serves, so each image is fetched once at one size.

use crate::{Emoji, Guild, Snowflake, StickerFormat, StickerItem, User};

pub const CDN: &str = "https://cdn.discordapp.com";
pub const MEDIA: &str = "https://media.discordapp.net";

/// Rounds a pixel size up to the nearest size the CDN serves.
pub fn cdn_size(px: u32) -> u32 {
    px.clamp(16, 4096).next_power_of_two()
}

fn is_animated(hash: &str) -> bool {
    hash.starts_with("a_")
}

/// `animate` false requests a still frame of an animated image, which saves decoding
/// every frame for avatars that are not hovered.
fn ext(hash: &str, animate: bool) -> &'static str {
    if animate && is_animated(hash) {
        "gif"
    } else {
        "webp"
    }
}

pub fn user_avatar(user: &User, px: u32, animate: bool) -> String {
    match user.avatar.as_deref() {
        Some(hash) => format!(
            "{CDN}/avatars/{}/{hash}.{}?size={}",
            user.id,
            ext(hash, animate),
            cdn_size(px)
        ),
        None => default_avatar(user.default_avatar_index()),
    }
}

pub fn default_avatar(index: u64) -> String {
    format!("{CDN}/embed/avatars/{index}.png")
}

pub fn member_avatar(
    guild_id: Snowflake,
    user_id: Snowflake,
    hash: &str,
    px: u32,
    animate: bool,
) -> String {
    format!(
        "{CDN}/guilds/{guild_id}/users/{user_id}/avatars/{hash}.{}?size={}",
        ext(hash, animate),
        cdn_size(px)
    )
}

pub fn user_banner(user_id: Snowflake, hash: &str, px: u32, animate: bool) -> String {
    format!(
        "{CDN}/banners/{user_id}/{hash}.{}?size={}",
        ext(hash, animate),
        cdn_size(px)
    )
}

pub fn avatar_decoration(asset: &str, px: u32) -> String {
    format!(
        "{CDN}/avatar-decoration-presets/{asset}.png?size={}&passthrough=true",
        cdn_size(px)
    )
}

pub fn guild_icon(guild: &Guild, px: u32, animate: bool) -> Option<String> {
    guild.icon.as_deref().map(|hash| {
        format!(
            "{CDN}/icons/{}/{hash}.{}?size={}",
            guild.id,
            ext(hash, animate),
            cdn_size(px)
        )
    })
}

pub fn guild_banner(guild: &Guild, px: u32) -> Option<String> {
    guild.banner.as_deref().map(|hash| {
        format!(
            "{CDN}/banners/{}/{hash}.{}?size={}",
            guild.id,
            ext(hash, true),
            cdn_size(px)
        )
    })
}

pub fn role_icon(role_id: Snowflake, hash: &str, px: u32) -> String {
    format!(
        "{CDN}/role-icons/{role_id}/{hash}.webp?size={}",
        cdn_size(px)
    )
}

pub fn badge_icon(icon: &str) -> String {
    format!("{CDN}/badge-icons/{icon}.png")
}

pub fn custom_emoji(id: Snowflake, animated: bool, px: u32) -> String {
    format!(
        "{CDN}/emojis/{id}.{}?size={}&quality=lossless",
        if animated { "gif" } else { "webp" },
        cdn_size(px)
    )
}

pub fn emoji(emoji: &Emoji, px: u32) -> Option<String> {
    emoji.id.map(|id| custom_emoji(id, emoji.animated, px))
}

/// Lottie stickers have no raster form on the CDN, so they return `None` and the caller
/// renders them through its Lottie path or shows the sticker's name.
pub fn sticker(item: &StickerItem, px: u32) -> Option<String> {
    let size = cdn_size(px);
    match item.format_type {
        StickerFormat::Png | StickerFormat::Apng => {
            Some(format!("{MEDIA}/stickers/{}.png?size={size}", item.id))
        }
        StickerFormat::Gif => Some(format!("{MEDIA}/stickers/{}.gif?size={size}", item.id)),
        StickerFormat::Lottie => None,
        StickerFormat::Unknown(_) => None,
    }
}

pub fn sticker_lottie(id: Snowflake) -> String {
    format!("{CDN}/stickers/{id}.json")
}

/// Resizes a `media.discordapp.net` proxy URL, so a 4000 px attachment is fetched at the
/// size it is drawn at.
pub fn sized_proxy_url(proxy_url: &str, width: u32, height: u32) -> String {
    let sep = if proxy_url.contains('?') { '&' } else { '?' };
    format!("{proxy_url}{sep}width={width}&height={height}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_round_up() {
        assert_eq!(cdn_size(0), 16);
        assert_eq!(cdn_size(40), 64);
        assert_eq!(cdn_size(128), 128);
        assert_eq!(cdn_size(9000), 4096);
    }

    #[test]
    fn avatars() {
        let mut u: User =
            serde_json::from_str(r#"{"id":"80351110224678912","username":"a"}"#).unwrap();
        assert_eq!(
            user_avatar(&u, 40, true),
            format!(
                "{CDN}/embed/avatars/{}.png",
                (80351110224678912u64 >> 22) % 6
            )
        );
        u.avatar = Some("a_123".into());
        assert!(user_avatar(&u, 40, true).ends_with("/a_123.gif?size=64"));
        assert!(user_avatar(&u, 40, false).ends_with("/a_123.webp?size=64"));
    }

    #[test]
    fn stickers() {
        let s = StickerItem {
            id: Snowflake::new(5).unwrap(),
            name: "x".into(),
            format_type: StickerFormat::Lottie,
        };
        assert_eq!(sticker(&s, 160), None);
    }
}

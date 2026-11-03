//! Discord's data types as rcord uses them: tolerant of fields Discord adds, compact in
//! memory, and shared by the gateway, REST client, cache and UI.

pub mod cdn;
mod channel;
mod emoji;
mod guild;
mod message;
mod permissions;
mod presence;
pub mod snowflake;
mod user;

pub use channel::{Channel, ChannelKind};
pub use emoji::{
    Emoji, Reaction, ReactionCounts, Sticker, StickerFormat, StickerItem, StickerPack,
};
pub use guild::{Guild, Member, Role, RoleColors};
pub use message::{
    Attachment, Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedMedia, EmbedProvider, Message,
    MessageKind, MessageReference, Nonce,
};
pub use permissions::{
    OverwriteKind, PermissionOverwrite, Permissions, base_permissions, channel_permissions,
};
pub use presence::{
    Activity, ActivityAssets, ActivityKind, ActivityTimestamps, ClientStatus, Presence,
    PresenceUser, Status, VoiceState,
};
pub use snowflake::Snowflake;
pub use user::{
    AvatarDecoration, ConnectedAccount, FlagBadge, MutualGuild, PremiumType, PrimaryGuild,
    ProfileBadge, ProfileMetadata, ProfileUser, Relationship, RelationshipKind, User, UserFlags,
    UserProfile,
};

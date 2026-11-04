//! Offline sample data, for trying the interface without an account and for screenshots.
//! Start with `RCORD_DEMO=1`. Nothing here touches the network; every name is invented.

use rcord_model::{Message, Snowflake};
use serde_json::{Value, json};

use crate::state::now_ms;

const GUILD: u64 = 1_100_000_000_000_000_001;
const ME: u64 = 1_100_000_000_000_000_100;
/// Spaces user IDs so each gets a different default avatar.
const STEP: u64 = 1 << 22;

/// Channel IDs, in sidebar order.
pub const GENERAL: u64 = GUILD + 25;

struct Person {
    id: u64,
    username: &'static str,
    global: &'static str,
    role: Option<u64>,
    status: &'static str,
    activity: Option<&'static str>,
}

const ROLE_MAINTAINER: u64 = 1_100_000_000_000_000_201;
const ROLE_CONTRIBUTOR: u64 = 1_100_000_000_000_000_202;

const PEOPLE: [Person; 8] = [
    Person {
        id: ME,
        username: "you",
        global: "You",
        role: None,
        status: "online",
        activity: None,
    },
    Person {
        id: ME + STEP,
        username: "mira",
        global: "Mira",
        role: Some(ROLE_MAINTAINER),
        status: "online",
        activity: Some("Reviewing pull requests"),
    },
    Person {
        id: ME + 2 * STEP,
        username: "oskar.dev",
        global: "Oskar",
        role: Some(ROLE_MAINTAINER),
        status: "dnd",
        activity: Some("Profiling the renderer"),
    },
    Person {
        id: ME + 3 * STEP,
        username: "juniper",
        global: "Juniper",
        role: Some(ROLE_CONTRIBUTOR),
        status: "online",
        activity: None,
    },
    Person {
        id: ME + 4 * STEP,
        username: "theo",
        global: "Theo",
        role: Some(ROLE_CONTRIBUTOR),
        status: "idle",
        activity: Some("Away for lunch"),
    },
    Person {
        id: ME + 5 * STEP,
        username: "kestrel",
        global: "Kestrel",
        role: None,
        status: "online",
        activity: None,
    },
    Person {
        id: ME + 6 * STEP,
        username: "vale",
        global: "Vale",
        role: None,
        status: "offline",
        activity: None,
    },
    Person {
        id: ME + 7 * STEP,
        username: "rowan",
        global: "Rowan",
        role: None,
        status: "offline",
        activity: None,
    },
];

fn user(p: &Person) -> Value {
    json!({
        "id": p.id.to_string(),
        "username": p.username,
        "global_name": p.global,
        "discriminator": "0",
        "avatar": null,
        "public_flags": if p.id == ME + STEP { 1 << 22 } else { 0 },
    })
}

fn channel(guild: u64, id: u64, kind: u8, name: &str, parent: Option<u64>, position: i32) -> Value {
    json!({
        "id": id.to_string(),
        "type": kind,
        "name": name,
        "guild_id": guild.to_string(),
        "parent_id": parent.map(|p| p.to_string()),
        "position": position,
        "permission_overwrites": [],
        "last_message_id": if id == GENERAL { Some(last_message_id().to_string()) } else { None },
    })
}

fn last_message_id() -> u64 {
    Snowflake::from_timestamp_ms(now_ms() - 60_000).map_or(0, Snowflake::get) + 99
}

pub fn ready() -> Value {
    let cat_info = GUILD + 10;
    let cat_text = GUILD + 11;
    let cat_voice = GUILD + 12;
    let channels = vec![
        channel(GUILD, cat_info, 4, "Information", None, 0),
        channel(GUILD, GUILD + 20, 0, "welcome", Some(cat_info), 0),
        channel(GUILD, GUILD + 21, 5, "announcements", Some(cat_info), 1),
        channel(GUILD, cat_text, 4, "Text Channels", None, 1),
        channel(GUILD, GENERAL, 0, "general", Some(cat_text), 0),
        channel(GUILD, GUILD + 22, 0, "showcase", Some(cat_text), 1),
        channel(GUILD, GUILD + 23, 0, "help", Some(cat_text), 2),
        channel(GUILD, GUILD + 24, 15, "ideas", Some(cat_text), 3),
        channel(GUILD, cat_voice, 4, "Voice", None, 2),
        channel(GUILD, GUILD + 30, 2, "Lounge", Some(cat_voice), 0),
        channel(GUILD, GUILD + 31, 2, "Pairing", Some(cat_voice), 1),
    ];
    let roles = json!([
        { "id": GUILD.to_string(), "name": "@everyone", "color": 0, "position": 0, "permissions": "2248473465835073", "hoist": false },
        { "id": ROLE_MAINTAINER.to_string(), "name": "Maintainers", "color": 0xE67E22, "position": 2, "permissions": "0", "hoist": true },
        { "id": ROLE_CONTRIBUTOR.to_string(), "name": "Contributors", "color": 0x3BA55C, "position": 1, "permissions": "0", "hoist": true },
    ]);
    let guild = |id: u64, name: &str, channels: Vec<Value>, roles: Value| {
        json!({
            "id": id.to_string(),
            "properties": { "id": id.to_string(), "name": name, "icon": null, "owner_id": (ME + STEP).to_string() },
            "channels": channels,
            "roles": roles,
            "emojis": [],
            "stickers": [],
            "member_count": PEOPLE.len(),
            "threads": [],
        })
    };
    let everyone = |id: u64| json!([{ "id": id.to_string(), "name": "@everyone", "color": 0, "position": 0, "permissions": "2248473465835073" }]);
    let small = |id: u64, name: &str| {
        guild(
            id,
            name,
            vec![channel(id, id + 1, 0, "general", None, 0)],
            everyone(id),
        )
    };
    let members: Vec<Value> = PEOPLE
        .iter()
        .map(|p| {
            json!({
                "user_id": p.id.to_string(),
                "roles": p.role.map(|r| vec![r.to_string()]).unwrap_or_default(),
                "joined_at": "2024-03-14T12:00:00+00:00",
            })
        })
        .collect();
    json!({
        "v": 9,
        "session_id": "demo",
        "user": user(&PEOPLE[0]),
        "users": PEOPLE.iter().skip(1).map(user).collect::<Vec<_>>(),
        "guilds": [
            guild(GUILD, "Pixel Forge", channels, roles),
            small(GUILD + 1000, "Night Shift"),
            small(GUILD + 2000, "Synth Club"),
            small(GUILD + 3000, "Trail Runners"),
        ],
        "merged_members": [members, [], [], []],
        "private_channels": [
            { "id": (GUILD + 5000).to_string(), "type": 1, "recipient_ids": [(ME + STEP).to_string()], "last_message_id": null },
            { "id": (GUILD + 5001).to_string(), "type": 1, "recipient_ids": [(ME + 3 * STEP).to_string()], "last_message_id": null },
        ],
        "relationships": PEOPLE.iter().skip(1).map(|p| json!({
            "id": p.id.to_string(),
            "type": if p.username == "vale" { 3 } else { 1 },
        })).collect::<Vec<_>>(),
        "read_state": { "entries": [] },
    })
}

/// A person's custom status, plus a game or song for a couple of them.
fn activities(p: &Person) -> Vec<Value> {
    let mut out: Vec<Value> = p
        .activity
        .map(|a| json!({ "type": 4, "name": "Custom Status", "state": a }))
        .into_iter()
        .collect();
    let now = now_ms();
    match p.username {
        "mira" => out.push(json!({
            "type": 0, "name": "Factorio", "details": "Building a main bus",
            "state": "Freeplay", "timestamps": { "start": now - 83 * 60_000 },
        })),
        "kestrel" => out.push(json!({
            "type": 2, "name": "Spotify", "details": "Midnight Drive", "state": "The Night Owls",
            "timestamps": { "start": now - 60_000, "end": now + 150_000 },
        })),
        _ => {}
    }
    out
}

pub fn presences() -> Value {
    json!({
        "merged_presences": {
            "friends": [],
            "guilds": [PEOPLE.iter().map(|p| json!({
                "user_id": p.id.to_string(),
                "status": p.status,
                "activities": activities(p),
            })).collect::<Vec<_>>(), [], [], []],
        },
        "guilds": [],
    })
}

pub fn member_list() -> Value {
    let mut items = Vec::new();
    let mut groups: Vec<(String, Vec<&Person>)> = vec![
        (ROLE_MAINTAINER.to_string(), Vec::new()),
        (ROLE_CONTRIBUTOR.to_string(), Vec::new()),
        ("online".into(), Vec::new()),
        ("offline".into(), Vec::new()),
    ];
    for p in &PEOPLE {
        let slot = match (p.status, p.role) {
            ("offline", _) => 3,
            (_, Some(ROLE_MAINTAINER)) => 0,
            (_, Some(ROLE_CONTRIBUTOR)) => 1,
            _ => 2,
        };
        groups[slot].1.push(p);
    }
    for (id, people) in &groups {
        if people.is_empty() {
            continue;
        }
        items.push(json!({ "group": { "id": id, "count": people.len() } }));
        for p in people {
            items.push(json!({ "member": {
                "user": user(p),
                "roles": p.role.map(|r| vec![r.to_string()]).unwrap_or_default(),
                "joined_at": "2024-03-14T12:00:00+00:00",
                "presence": {
                    "user": { "id": p.id.to_string() },
                    "status": p.status,
                    "activities": activities(p),
                },
            }}));
        }
    }
    let online = PEOPLE.iter().filter(|p| p.status != "offline").count();
    json!({
        "guild_id": GUILD.to_string(),
        "id": "everyone",
        "member_count": PEOPLE.len(),
        "online_count": online,
        "groups": groups.iter().filter(|g| !g.1.is_empty()).map(|g| json!({ "id": g.0, "count": g.1.len() })).collect::<Vec<_>>(),
        "ops": [{ "op": "SYNC", "range": [0, 99], "items": items }],
    })
}

/// The conversation in #general, newest first, as the API returns it.
pub fn general_history() -> Vec<Message> {
    let now = now_ms();
    let lines: [(u64, u64, &str); 11] = [
        (
            1,
            52,
            "Morning! The new message list landed on main. It only lays out the rows on screen now, so scrolling a channel with 50k messages stays smooth.",
        ),
        (
            1,
            52,
            "Memory sits flat too: old pages are dropped and reload when you scroll back.",
        ),
        (
            3,
            48,
            "That is a huge difference. Tried it on the big announcements channel and it doesn't even blink.",
        ),
        (
            2,
            41,
            "Profiled it on my laptop: **under 2 ms** per frame while flinging the scrollbar.",
        ),
        (
            2,
            41,
            "```rust\nlet rows = list.visible_range();\nfor ix in rows {\n    cx.paint(row(ix));\n}\n```",
        ),
        (
            4,
            33,
            "Is the panel layout saved per window or per account?",
        ),
        (
            1,
            31,
            "Per install. Hold right click on any panel, drag it where you want it, and let go. It sticks.",
        ),
        (
            5,
            18,
            "Just moved the member list to the left. ||Never going back.||",
        ),
        (3, 9, "Can we get a compact mode screenshot for the README?"),
        (
            1,
            4,
            "Already in settings → Appearance. Try *Compact* with avatars on.",
        ),
        (
            2,
            1,
            "Shipping the release notes in a bit. Ping me if anything looks off <@1100000000000000100>.",
        ),
    ];
    let mut out: Vec<Message> = lines
        .iter()
        .enumerate()
        .map(|(ix, (who, mins_ago, text))| {
            let p = &PEOPLE[*who as usize];
            let ms = now - mins_ago * 60_000 + ix as u64 * 1_500;
            let id = Snowflake::from_timestamp_ms(ms).map_or(0, Snowflake::get) + ix as u64;
            let ts = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
                .ok()
                .and_then(|d| {
                    d.format(&time::format_description::well_known::Rfc3339)
                        .ok()
                })
                .unwrap_or_default();
            let reactions = match ix {
                0 => json!([
                    { "count": 6, "me": true, "emoji": { "id": null, "name": "🎉" } },
                    { "count": 3, "me": false, "emoji": { "id": null, "name": "🚀" } },
                ]),
                3 => json!([{ "count": 4, "me": false, "emoji": { "id": null, "name": "🔥" } }]),
                7 => json!([{ "count": 2, "me": true, "emoji": { "id": null, "name": "😂" } }]),
                _ => json!([]),
            };
            let mentions = if text.contains("<@") {
                json!([user(&PEOPLE[0])])
            } else {
                json!([])
            };
            json!({
                "id": id.to_string(),
                "channel_id": GENERAL.to_string(),
                "guild_id": GUILD.to_string(),
                "author": user(p),
                "content": text,
                "timestamp": ts,
                "reactions": reactions,
                "mentions": mentions,
                "type": 0,
            })
        })
        .filter_map(|v| {
            serde_json::from_value(v)
                .map_err(|e| tracing::warn!("demo message: {e}"))
                .ok()
        })
        .collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(name: &str, v: Value) -> rcord_store::Update {
        let raw = serde_json::value::to_raw_value(&v).unwrap();
        rcord_store::parse(name, &raw).unwrap()
    }

    #[test]
    fn sample_data_parses() {
        let mut store = rcord_store::Store::default();
        store.apply(parse("READY", ready()), now_ms());
        store.apply(parse("READY_SUPPLEMENTAL", presences()), now_ms());
        store.apply(parse("GUILD_MEMBER_LIST_UPDATE", member_list()), now_ms());
        assert_eq!(store.guilds().count(), 4);
        let g = Snowflake::new(GUILD).unwrap();
        assert!(
            store
                .member_list(g)
                .is_some_and(|l| l.rows.len() >= PEOPLE.len())
        );
        assert_eq!(general_history().len(), 11);
    }
}

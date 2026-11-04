use serde_json::json;
use serde_json::value::to_raw_value;

use super::*;

fn id(n: u64) -> Snowflake {
    Snowflake::new(n).unwrap()
}

fn apply(store: &mut Store, name: &str, data: serde_json::Value) -> Change {
    let raw = to_raw_value(&data).unwrap();
    store.apply(parse(name, &raw).unwrap(), 1_000_000)
}

fn ready() -> Store {
    let mut store = Store::new(50);
    apply(
        &mut store,
        "READY",
        json!({
            "session_id": "s",
            "user": {"id": "1", "username": "me"},
            "users": [{"id": "2", "username": "friend", "global_name": "Friend"}],
            "guilds": [{
                "id": "100",
                "properties": {"name": "Rust Club", "owner_id": "9"},
                "roles": [
                    {"id": "100", "name": "@everyone", "permissions": "3072"},
                    {"id": "101", "name": "mods", "permissions": "0", "color": 16711680, "position": 1}
                ],
                "channels": [
                    {"id": "200", "type": 4, "name": "Text", "position": 0},
                    {"id": "201", "type": 0, "name": "general", "parent_id": "200", "position": 0, "last_message_id": "500"},
                    {"id": "202", "type": 0, "name": "secret", "parent_id": "200", "position": 1,
                     "permission_overwrites": [{"id": "100", "type": 0, "allow": "0", "deny": "1024"}]},
                    {"id": "203", "type": 2, "name": "Voice", "parent_id": "200", "position": 0},
                    {"id": "204", "type": 0, "name": "rules", "position": 0}
                ]
            }],
            "merged_members": [[{"user_id": "1", "roles": ["101"], "nick": "Captain"}]],
            "private_channels": [
                {"id": "300", "type": 1, "recipient_ids": ["2"], "last_message_id": "400"},
                {"id": "301", "type": 1, "recipient_ids": ["2"], "last_message_id": "450"}
            ],
            "relationships": [{"id": "2", "type": 1}],
            "read_state": {"entries": [
                {"id": "201", "last_message_id": "499", "mention_count": 2},
                {"id": "300", "last_message_id": 0}
            ], "version": 1, "partial": false}
        }),
    );
    store
}

fn message(id_: u64, channel: u64, author: u64, content: &str) -> serde_json::Value {
    json!({
        "id": id_.to_string(),
        "channel_id": channel.to_string(),
        "author": {"id": author.to_string(), "username": format!("u{author}")},
        "content": content,
        "timestamp": "2024-01-01T00:00:00+00:00"
    })
}

#[test]
fn ready_builds_everything() {
    let s = ready();
    assert_eq!(s.me_id(), Some(id(1)));
    assert_eq!(s.guilds().count(), 1);
    assert_eq!(s.display_name(Some(id(100)), id(1)), Some("Captain"));
    assert_eq!(s.name_color(id(100), id(1)), Some(0xFF0000));
    assert_eq!(s.friends().count(), 1);
    // Most recent DM first.
    let dms: Vec<_> = s.private_channels().map(|c| c.id).collect();
    assert_eq!(dms, vec![id(301), id(300)]);
    assert_eq!(s.channel_name(id(300)), "Friend");
    assert!(s.is_unread(id(201)));
    assert_eq!(s.read_state(id(201)).mentions, 2);
    assert_eq!(s.guild_unread(id(100)), (true, 2));
}

#[test]
fn sidebar_hides_what_cannot_be_seen() {
    let s = ready();
    let groups = s.channel_groups(id(100));
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].category, None);
    assert_eq!(groups[0].channels, vec![id(204)]);
    assert_eq!(groups[1].category, Some(id(200)));
    // Text before voice, and #secret is hidden by its overwrite.
    assert_eq!(groups[1].channels, vec![id(201), id(203)]);
}

#[test]
fn messages_flow() {
    let mut s = ready();
    let page: Vec<Message> = (1..=3)
        .rev()
        .map(|n| serde_json::from_value(message(600 + n, 201, 2, "hi")).unwrap())
        .collect();
    s.insert_history(id(201), page, 50, true);
    let list = s.messages(id(201)).unwrap();
    assert!(list.reached_start && list.live);
    assert_eq!(
        list.messages.iter().map(|m| m.id.get()).collect::<Vec<_>>(),
        vec![601, 602, 603]
    );

    s.add_pending(id(201), "n1".into(), "sending".into());
    let mut mine = message(700, 201, 1, "sending");
    mine["nonce"] = json!("n1");
    assert_eq!(
        apply(&mut s, "MESSAGE_CREATE", mine),
        Change::Messages(id(201))
    );
    assert!(s.pending(id(201)).is_empty());
    assert!(!s.is_unread(id(201)));

    apply(
        &mut s,
        "MESSAGE_UPDATE",
        json!({"id": "700", "channel_id": "201", "content": "edited", "edited_timestamp": "x"}),
    );
    assert_eq!(s.message(id(201), id(700)).unwrap().content, "edited");
    assert_eq!(s.message(id(201), id(700)).unwrap().author.id, id(1));

    apply(
        &mut s,
        "MESSAGE_DELETE",
        json!({"id": "601", "channel_id": "201"}),
    );
    assert!(s.message(id(201), id(601)).is_none());
}

#[test]
fn messages_outside_a_loaded_window_are_not_appended() {
    let mut s = ready();
    apply(&mut s, "MESSAGE_CREATE", message(800, 201, 2, "hello"));
    assert!(s.messages(id(201)).is_none());
    assert_eq!(s.channel(id(201)).unwrap().last_message_id, Some(id(800)));
    assert!(s.is_unread(id(201)));
}

#[test]
fn window_is_capped() {
    let mut s = ready();
    s.insert_history(id(201), Vec::new(), 50, true);
    for n in 0..60 {
        apply(&mut s, "MESSAGE_CREATE", message(1_000 + n, 201, 2, "x"));
    }
    let list = s.messages(id(201)).unwrap();
    assert_eq!(list.messages.len(), 50);
    assert_eq!(list.messages.front().unwrap().id, id(1_010));
    assert!(!list.reached_start);
}

#[test]
fn reactions_count_and_track_me() {
    let mut s = ready();
    s.insert_history(
        id(201),
        vec![serde_json::from_value(message(900, 201, 2, "x")).unwrap()],
        50,
        true,
    );
    let r = |user: &str| json!({"user_id": user, "channel_id": "201", "message_id": "900", "emoji": {"id": null, "name": "🔥"}});
    apply(&mut s, "MESSAGE_REACTION_ADD", r("1"));
    apply(&mut s, "MESSAGE_REACTION_ADD", r("2"));
    let m = s.message(id(201), id(900)).unwrap();
    assert_eq!(m.reactions[0].count, 2);
    assert!(m.reactions[0].me);
    apply(&mut s, "MESSAGE_REACTION_REMOVE", r("1"));
    apply(&mut s, "MESSAGE_REACTION_REMOVE", r("2"));
    assert!(s.message(id(201), id(900)).unwrap().reactions.is_empty());
}

#[test]
fn member_list_ops() {
    let mut s = ready();
    let member = |uid: &str, status: &str| json!({"member": {"user": {"id": uid, "username": uid}, "roles": [], "presence": {"status": status, "activities": []}}});
    apply(
        &mut s,
        "GUILD_MEMBER_LIST_UPDATE",
        json!({"guild_id": "100", "id": "everyone", "member_count": 3, "online_count": 2,
        "groups": [{"id": "online", "count": 2}],
        "ops": [{"op": "SYNC", "range": [0, 99], "items": [
            {"group": {"id": "online", "count": 2}}, member("7", "online"), member("8", "idle")
        ]}]}),
    );
    let list = s.member_list(id(100)).unwrap();
    assert_eq!(list.rows.len(), 3);
    assert_eq!(s.status(id(8)), Status::Idle);
    apply(
        &mut s,
        "GUILD_MEMBER_LIST_UPDATE",
        json!({"guild_id": "100", "id": "everyone", "ops": [
            {"op": "DELETE", "index": 1},
            {"op": "INSERT", "index": 2, "item": member("9", "dnd")}
        ]}),
    );
    let rows: Vec<_> = s
        .member_list(id(100))
        .unwrap()
        .rows
        .iter()
        .filter_map(|r| match r {
            Some(MemberRow::Member(u)) => Some(u.get()),
            _ => None,
        })
        .collect();
    assert_eq!(rows, vec![8, 9]);
}

#[test]
fn typing_expires() {
    let mut s = ready();
    apply(
        &mut s,
        "TYPING_START",
        json!({"channel_id": "201", "user_id": "2", "timestamp": 1000}),
    );
    assert_eq!(s.typing(id(201), 1_000_000), vec![id(2)]);
    assert!(s.typing(id(201), 1_000_000 + TYPING_MS + 1).is_empty());
}

#[test]
fn unknown_events_are_ignored() {
    let mut s = ready();
    assert_eq!(
        apply(&mut s, "SOMETHING_NEW", json!({"x": 1})),
        Change::None
    );
}

#[test]
fn voice_server_update_is_parsed_not_cached() {
    let raw = to_raw_value(&json!({
        "token": "abc",
        "guild_id": "100",
        "endpoint": "c-ams10-1234.discord.media:443"
    }))
    .unwrap();
    let Update::VoiceServerUpdate(v) = parse("VOICE_SERVER_UPDATE", &raw).unwrap() else {
        panic!("expected a voice server update");
    };
    assert_eq!(v.guild_id, Some(id(100)));
    assert_eq!(
        v.endpoint.as_deref(),
        Some("c-ams10-1234.discord.media:443")
    );

    // A server reallocation sends no endpoint, and a direct call names the channel.
    let raw = to_raw_value(&json!({"token": "t", "channel_id": "5", "endpoint": null})).unwrap();
    let Update::VoiceServerUpdate(v) = parse("VOICE_SERVER_UPDATE", &raw).unwrap() else {
        panic!("expected a voice server update");
    };
    assert_eq!(
        (v.guild_id, v.channel_id, v.endpoint),
        (None, Some(id(5)), None)
    );

    let mut store = ready();
    assert_eq!(
        apply(
            &mut store,
            "VOICE_SERVER_UPDATE",
            json!({"token": "t", "guild_id": "100"})
        ),
        Change::None
    );
}

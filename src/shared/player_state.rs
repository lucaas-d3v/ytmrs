//! Port of `~shared/playerstatestore/types` plus the YTM data mapping from the
//! main process PlayerStateStore.

use serde::{Serialize, Serializer};
use serde_json::{json, Value};

use super::{js_parse_int, ser_num, ser_opt_num};

macro_rules! int_enum {
    ($name:ident { $($variant:ident = $val:expr),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),* }
        impl $name {
            pub fn as_i64(&self) -> i64 { match self { $(Self::$variant => $val),* } }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_i64(self.as_i64()) }
        }
    };
}

int_enum!(VideoState { Unknown = -1, Paused = 0, Playing = 1, Buffering = 2 });
int_enum!(RepeatMode { Unknown = -1, None = 0, All = 1, One = 2 });
int_enum!(LikeStatus { Unknown = -1, Dislike = 0, Indifferent = 1, Like = 2 });
int_enum!(VideoType { Unknown = -1, MusicAudio = 0, MusicVideo = 1, MusicUploaded = 2, PodcastEpisode = 3 });

impl RepeatMode {
    pub fn from_i64(v: i64) -> Option<Self> {
        match v {
            -1 => Some(Self::Unknown),
            0 => Some(Self::None),
            1 => Some(Self::All),
            2 => Some(Self::One),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Thumbnail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_num")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_num")]
    pub height: Option<f64>,
}

impl Thumbnail {
    pub fn w(&self) -> f64 {
        self.width.unwrap_or(f64::NAN)
    }
    pub fn h(&self) -> f64 {
        self.height.unwrap_or(f64::NAN)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoDetails {
    pub album: Option<String>,
    pub album_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(serialize_with = "ser_num")]
    pub duration_seconds: f64,
    pub thumbnails: Vec<Thumbnail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub like_status: LikeStatus,
    pub video_type: VideoType,
    pub is_live: bool,
}

impl VideoDetails {
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or("")
    }
    pub fn author(&self) -> &str {
        self.author.as_deref().unwrap_or("")
    }
    pub fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerQueueItem {
    pub thumbnails: Vec<Thumbnail>,
    pub title: String,
    pub author: String,
    pub duration: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    pub counterparts: Option<Vec<PlayerQueueItem>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerQueue {
    pub automix_items: Vec<Option<PlayerQueueItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoplay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_generating: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_infinite: Option<bool>,
    pub items: Vec<Option<PlayerQueueItem>>,
    pub repeat_mode: RepeatMode,
    pub selected_item_index: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shuffle_enabled: Option<bool>,
}

impl PlayerQueue {
    pub fn shuffle(&self) -> bool {
        self.shuffle_enabled.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub video_details: Option<VideoDetails>,
    pub playlist_id: Option<String>,
    pub track_state: VideoState,
    pub queue: Option<PlayerQueue>,
    #[serde(serialize_with = "ser_num")]
    pub video_progress: f64,
    #[serde(serialize_with = "ser_num")]
    pub volume: f64,
    pub muted: bool,
    pub ad_playing: bool,
    pub has_full_metadata: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            video_details: None,
            playlist_id: None,
            track_state: VideoState::Unknown,
            queue: None,
            video_progress: 0.0,
            volume: 0.0,
            muted: false,
            ad_playing: false,
            has_full_metadata: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerSignal {
    NewPlayback,
}

// ---- YTM data mapping ----

fn str_of(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.to_string())
}

fn opt_num(v: &Value) -> Option<f64> {
    v.as_f64()
}

pub fn get_ytm_text_run(runs: &Value) -> String {
    runs.as_array()
        .map(|runs| runs.iter().filter_map(|r| r.get("text").and_then(|t| t.as_str())).collect::<String>())
        .unwrap_or_default()
}

fn text_of(v: &Value) -> String {
    match v.get("runs") {
        Some(runs) if runs.is_array() => get_ytm_text_run(runs),
        _ => String::new(),
    }
}

pub fn map_thumbnails(thumbnails: &Value) -> Vec<Thumbnail> {
    thumbnails
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| Thumbnail { url: str_of(&t["url"]), width: opt_num(&t["width"]), height: opt_num(&t["height"]) })
                .collect()
        })
        .unwrap_or_default()
}

fn transform_playlist_panel_video_renderer(r: &Value, counterpart: Option<&Value>) -> PlayerQueueItem {
    PlayerQueueItem {
        thumbnails: if r["thumbnail"].is_object() { map_thumbnails(&r["thumbnail"]["thumbnails"]) } else { vec![] },
        title: text_of(&r["title"]),
        author: text_of(&r["shortBylineText"]),
        duration: text_of(&r["lengthText"]),
        selected: r["selected"].as_bool(),
        video_id: str_of(&r["videoId"]),
        counterparts: counterpart.and_then(|c| c.as_array()).map(|c| {
            c.iter()
                .map(|cp| transform_playlist_panel_video_renderer(&cp["counterpartRenderer"]["playlistPanelVideoRenderer"], None))
                .collect()
        }),
    }
}

pub fn map_queue_item(item: &Value) -> Option<PlayerQueueItem> {
    if item["playlistPanelVideoRenderer"].is_object() {
        Some(transform_playlist_panel_video_renderer(&item["playlistPanelVideoRenderer"], None))
    } else if item["playlistPanelVideoWrapperRenderer"].is_object() {
        let w = &item["playlistPanelVideoWrapperRenderer"];
        let r = &w["primaryRenderer"]["playlistPanelVideoRenderer"];
        if !r.is_object() {
            return None;
        }
        Some(transform_playlist_panel_video_renderer(r, Some(&w["counterpart"])))
    } else {
        None
    }
}

pub fn transform_repeat_mode(v: &Value) -> RepeatMode {
    match v.as_str() {
        Some("NONE") => RepeatMode::None,
        Some("ALL") => RepeatMode::All,
        Some("ONE") => RepeatMode::One,
        _ => RepeatMode::Unknown,
    }
}

pub fn transform_like_status(v: &str) -> LikeStatus {
    match v {
        "DISLIKE" => LikeStatus::Dislike,
        "INDIFFERENT" => LikeStatus::Indifferent,
        "LIKE" => LikeStatus::Like,
        _ => LikeStatus::Unknown,
    }
}

pub fn transform_video_type(v: &Value) -> VideoType {
    match v.as_str() {
        Some("MUSIC_VIDEO_TYPE_ATV") => VideoType::MusicAudio,
        Some("MUSIC_VIDEO_TYPE_OMV") | Some("MUSIC_VIDEO_TYPE_UGC") => VideoType::MusicVideo,
        Some("MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK") => VideoType::MusicUploaded,
        Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE") => VideoType::PodcastEpisode,
        _ => VideoType::Unknown,
    }
}

/// YTM player state: Unstarted -1, Ended 0, Playing 1, Paused 2, Buffering 3, Cued 5
pub fn transform_video_state(v: &Value) -> VideoState {
    match v.as_i64() {
        Some(2) => VideoState::Paused,
        Some(1) => VideoState::Playing,
        Some(3) => VideoState::Buffering,
        _ => VideoState::Unknown,
    }
}

pub fn video_details_from_ytm(details: &Value, album: &Value, like_status: &Value) -> VideoDetails {
    VideoDetails {
        author: str_of(&details["author"]),
        channel_id: str_of(&details["channelId"]),
        title: str_of(&details["title"]),
        album: album.get("text").and_then(str_of_ref),
        album_id: album.get("id").and_then(str_of_ref),
        like_status: transform_like_status(like_status.as_str().unwrap_or("")),
        thumbnails: if details["thumbnail"].is_object() { map_thumbnails(&details["thumbnail"]["thumbnails"]) } else { vec![] },
        duration_seconds: js_parse_int(&details["lengthSeconds"]),
        id: str_of(&details["videoId"]),
        video_type: transform_video_type(&details["musicVideoType"]),
        is_live: details["isLive"].as_bool().unwrap_or(false),
    }
}

fn str_of_ref(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.to_string())
}

pub fn queue_from_ytm(queue: &Value) -> Option<PlayerQueue> {
    if queue.is_null() || !queue.is_object() {
        return None;
    }
    let items: Vec<Option<PlayerQueueItem>> =
        queue["items"].as_array().map(|a| a.iter().map(map_queue_item).collect()).unwrap_or_default();
    let automix: Vec<Option<PlayerQueueItem>> =
        queue["automixItems"].as_array().map(|a| a.iter().map(map_queue_item).collect()).unwrap_or_default();
    let selected = items
        .iter()
        .position(|i| i.as_ref().and_then(|i| i.selected).unwrap_or(false))
        .map(|i| i as i64)
        .unwrap_or(-1);
    Some(PlayerQueue {
        automix_items: automix,
        autoplay: queue["autoplay"].as_bool(),
        is_generating: queue["isGenerating"].as_bool(),
        is_infinite: queue["isInfinite"].as_bool(),
        items,
        repeat_mode: transform_repeat_mode(&queue["repeatMode"]),
        selected_item_index: selected,
        shuffle_enabled: queue["shuffleEnabled"].as_bool(),
    })
}

/// Companion server API v1 representation of the player state
pub fn transform_player_state_v1(state: &PlayerState) -> Value {
    let queue = state.queue.as_ref().map(|q| {
        let mut o = serde_json::Map::new();
        if let Some(v) = q.autoplay {
            o.insert("autoplay".into(), json!(v));
        }
        o.insert("items".into(), serde_json::to_value(&q.items).unwrap_or(Value::Null));
        o.insert("automixItems".into(), serde_json::to_value(&q.automix_items).unwrap_or(Value::Null));
        if let Some(v) = q.is_generating {
            o.insert("isGenerating".into(), json!(v));
        }
        if let Some(v) = q.is_infinite {
            o.insert("isInfinite".into(), json!(v));
        }
        o.insert("repeatMode".into(), json!(q.repeat_mode.as_i64()));
        o.insert("selectedItemIndex".into(), json!(q.selected_item_index));
        if let Some(v) = q.shuffle_enabled {
            o.insert("shuffleEnabled".into(), json!(v));
        }
        Value::Object(o)
    });
    let num = |v: f64| serde_json::to_value(NumWrap(v)).unwrap_or(Value::Null);
    let video = state.video_details.as_ref().map(|v| {
        let mut o = serde_json::Map::new();
        if let Some(a) = &v.author {
            o.insert("author".into(), json!(a));
        }
        if let Some(c) = &v.channel_id {
            o.insert("channelId".into(), json!(c));
        }
        if let Some(t) = &v.title {
            o.insert("title".into(), json!(t));
        }
        o.insert("album".into(), json!(v.album));
        o.insert("albumId".into(), json!(v.album_id));
        o.insert("likeStatus".into(), json!(v.like_status.as_i64()));
        o.insert("thumbnails".into(), serde_json::to_value(&v.thumbnails).unwrap_or(Value::Null));
        o.insert("durationSeconds".into(), num(v.duration_seconds));
        if let Some(id) = &v.id {
            o.insert("id".into(), json!(id));
        }
        o.insert("isLive".into(), json!(v.is_live));
        o.insert("videoType".into(), json!(v.video_type.as_i64()));
        o.insert("metadataFilled".into(), json!(state.has_full_metadata));
        Value::Object(o)
    });
    json!({
        "player": {
            "trackState": state.track_state.as_i64(),
            "videoProgress": num(state.video_progress),
            "volume": num(state.volume),
            "muted": state.muted,
            "adPlaying": state.ad_playing,
            "queue": queue
        },
        "video": video,
        "playlistId": state.playlist_id
    })
}

struct NumWrap(f64);
impl Serialize for NumWrap {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ser_num(&self.0, s)
    }
}

pub fn highest_res_thumbnail_reduce(thumbnails: &[Thumbnail]) -> Option<String> {
    // Equivalent of `thumbnails.reduce((acc, cur) => cur.h*cur.w <= acc.h*acc.w ? acc : cur, thumbnails[0]).url`
    let first = thumbnails.first()?;
    let mut acc = first;
    for cur in thumbnails {
        // NaN comparisons are false -> picks current, same as JS
        if cur.h() * cur.w() <= acc.h() * acc.w() {
            continue;
        }
        acc = cur;
    }
    acc.url.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_queue_and_details() {
        let queue = json!({
            "items": [
                {"playlistPanelVideoRenderer": {"title": {"runs": [{"text": "A"}, {"text": "B"}]}, "selected": false, "videoId": "x"}},
                {"playlistPanelVideoWrapperRenderer": {"primaryRenderer": {"playlistPanelVideoRenderer": {"title": {"runs": [{"text": "C"}]}, "selected": true, "videoId": "y"}}, "counterpart": [{"counterpartRenderer": {"playlistPanelVideoRenderer": {"videoId": "z"}}}]}},
                {}
            ],
            "automixItems": [],
            "repeatMode": "ALL",
            "shuffleEnabled": true
        });
        let q = queue_from_ytm(&queue).unwrap();
        assert_eq!(q.items.len(), 3);
        assert_eq!(q.items[0].as_ref().unwrap().title, "AB");
        assert_eq!(q.selected_item_index, 1);
        assert!(q.items[2].is_none());
        assert_eq!(q.repeat_mode, RepeatMode::All);
        assert_eq!(q.items[1].as_ref().unwrap().counterparts.as_ref().unwrap()[0].video_id.as_deref(), Some("z"));

        let d = video_details_from_ytm(&json!({"title": "T", "lengthSeconds": "215", "videoId": "v", "musicVideoType": "MUSIC_VIDEO_TYPE_OMV"}), &json!({"id": "MPRE", "text": "Album"}), &json!("LIKE"));
        assert_eq!(d.duration_seconds, 215.0);
        assert_eq!(d.video_type, VideoType::MusicVideo);
        assert_eq!(d.like_status, LikeStatus::Like);
        assert_eq!(d.album.as_deref(), Some("Album"));
    }

    #[test]
    fn v1_transform_shape() {
        let mut s = PlayerState::default();
        s.video_progress = 12.5;
        s.volume = 50.0;
        let v = transform_player_state_v1(&s);
        assert_eq!(v["player"]["videoProgress"], json!(12.5));
        assert_eq!(v["player"]["volume"].to_string(), "50");
        assert_eq!(v["player"]["trackState"], json!(-1));
        assert!(v["video"].is_null());
    }

    #[test]
    fn thumbnail_reduce() {
        let t = vec![
            Thumbnail { url: Some("a".into()), width: Some(60.0), height: Some(60.0) },
            Thumbnail { url: Some("b".into()), width: Some(544.0), height: Some(544.0) },
            Thumbnail { url: Some("c".into()), width: Some(120.0), height: Some(120.0) },
        ];
        assert_eq!(highest_res_thumbnail_reduce(&t).as_deref(), Some("b"));
        assert_eq!(highest_res_thumbnail_reduce(&[]), None);
    }
}

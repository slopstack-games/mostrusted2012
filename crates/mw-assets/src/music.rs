//! Music data: the song lists of `UI/SONGS/SONGS.BNDL` and the game's streamed waves (docs/formats.md
//! "Music"; the song manager's code is in docs/decomp.md "Music").
//!
//! `Genesys.Gen.SongList` (36 B; resource id = 0x01 << 56 | UniqueId):
//!
//! | off | field | |
//! |---|---|---|
//! | 0x0c | `f6b7d3df` | string: a Spotify playlist URL on list 1084858, else empty |
//! | 0x14 | `FeedbackMessage` | UniqueId (30567 on the three licensed lists) |
//! | 0x18 | `UniqueId` | |
//! | 0x1c | `Songs` | handles of `Song`s |
//! | 0x20 | count | u16 |
//! | 0x22 | `341d0a95` | bool: true on the three licensed lists (1622455, 1836280, 1845669) |
//! | 0x23 | `3fdcf508` | bool |
//!
//! `Genesys.Gen.Song` (48 B): +0x0c `Sequence` (handle, unused in the data), +0x14 `Sound` (a `Wave` resource),
//! +0x1c `c95dc017` (UniqueId, 0), +0x20 `Artist` (language id), +0x24 `UniqueId`, +0x28 `e4b95981` (language
//! id: the track title), +0x2c `bc7081c6` (f32, −0.92..0.99).
//!
//! `Wave` resources (header): +0x00 u32 = 0x80 when an SPS prefetch follows the 0x80-byte header, +0x10 f32
//! (1.0 on every song, 0.499 on the 14729 / 14730 loops: our reading, a gain), +0x14 f32 duration in ms, +0x18
//! u32 prefetch length; without a prefetch, +0x28 is the stream's path (`UI\SONGS\14729.SPS`). A prefetched
//! wave's prefetch is the stream's `H` block and first `D` blocks; the rest of the stream is the file
//! `<UniqueId>.SPS` (e.g. `UI/SEQUENCES/STREAMS/1772189.SPS`, which starts with a `D` block).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use mw_audio::Pcm;
use mw_bundle::ResourceId;
use mw_formats::genesys::{Value, field_hash};

use crate::{AssetStore, Error, Result};

/// One song of a list.
#[derive(Debug, Clone, PartialEq)]
pub struct Song {
    pub id: u32,
    /// The `Wave` resource that streams it.
    pub wave: ResourceId,
    /// Language ids.
    pub artist: u32,
    pub title: u32,
    /// `bc7081c6` (+0x2c): what it is used for is not traced.
    pub bc7081c6: f32,
}

/// A `SongList`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SongList {
    pub id: u32,
    pub feedback_message: u32,
    pub songs: Vec<Song>,
    /// `341d0a95` (+0x22).
    pub flag_22: bool,
    /// `3fdcf508` (+0x23).
    pub flag_23: bool,
    pub url: String,
}

/// The list the boot code loads first (`FUN_007a4010` state 3 loads GeneSys object 0x1c70a).
pub const BOOT_SONG_LIST: u32 = 116_490;

fn uint(v: Option<&Value>) -> u32 {
    match v {
        Some(Value::UInt(u)) => *u as u32,
        Some(Value::Int(i)) => *i as u32,
        _ => 0,
    }
}

fn handles(v: Option<&Value>) -> Vec<ResourceId> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::Handle(Some(id)) | Value::Ref(id) => Some(*id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn handle(v: Option<&Value>) -> Option<ResourceId> {
    match v {
        Some(Value::Handle(Some(id))) | Some(Value::Ref(id)) => Some(*id),
        _ => None,
    }
}

/// Every `.SPS` file of the install by its stem (`1772189` → `UI/SEQUENCES/STREAMS/1772189.SPS`).
fn sps_index(store: &AssetStore) -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| {
        store
            .game()
            .files_with_extension("SPS")
            .into_iter()
            .filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_ascii_uppercase(), store.game().root().join(&p))))
            .collect()
    })
}

/// The bytes of an SPS stream's blocks from the start of `data` (a prefetch padded after its last block).
fn blocks_len(data: &[u8]) -> usize {
    let mut o = 0;
    while o + 4 <= data.len() {
        let size = u32::from_be_bytes([0, data[o + 1], data[o + 2], data[o + 3]]) as usize;
        if size < 4 || !matches!(data[o], b'H' | b'D' | b'E') || o + size > data.len() {
            break;
        }
        o += size;
        if data[o - size] == b'E' {
            break;
        }
    }
    o
}

impl AssetStore {
    /// A `SongList` by UniqueId, with its songs.
    pub fn song_list(&self, id: u32) -> Result<SongList> {
        let o = self.genesys_object(ResourceId(0x0100_0000_0000_0000 | id as u64))?;
        if o.type_name != "Genesys.Gen.SongList" {
            return Err(Error::Missing(ResourceId(id as u64)));
        }
        let songs = handles(o.get(field_hash("Songs")))
            .into_iter()
            .filter_map(|s| {
                let s = self.genesys_object(s).ok()?;
                Some(Song {
                    id: uint(s.get(0x655d_3f5f)),
                    wave: handle(s.get(field_hash("Sound")))?,
                    artist: uint(s.get(field_hash("Artist"))),
                    title: uint(s.get(0xe4b9_5981)),
                    bc7081c6: match s.get(0xbc70_81c6) {
                        Some(Value::Float(f)) => *f,
                        _ => 0.0,
                    },
                })
            })
            .collect();
        Ok(SongList {
            id,
            feedback_message: uint(o.get(field_hash("FeedbackMessage"))),
            songs,
            flag_22: matches!(o.get(0x341d_0a95), Some(Value::Bool(true))),
            flag_23: matches!(o.get(0x3fdc_f508), Some(Value::Bool(true))),
            url: match o.get(0xf6b7_d3df) {
                Some(Value::Str(s)) => s.clone(),
                _ => String::new(),
            },
        })
    }

    /// Decode a `Wave` resource's whole stream: its prefetch followed by the streamed file, or the file its
    /// header names. Returns the PCM, the header's gain (+0x10) and whether the stream loops (its `H` block's
    /// loop flag: the title's loop 1772190).
    pub fn wave_stream(&self, id: ResourceId) -> Result<(Pcm, f32, bool)> {
        let res = self.load(id)?;
        let h = res.header();
        let le = |o: usize| h.get(o..o + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
        let gain = f32::from_bits(le(0x10));
        let gain = if gain.is_finite() && gain > 0.0 { gain } else { 1.0 };
        let mut data = Vec::new();
        if le(0) == 0x80 && h.get(0x80) == Some(&b'H') {
            let pre = &h[0x80..];
            let n = blocks_len(pre);
            data.extend_from_slice(&pre[..n]);
            if !ends_with_end_block(&data) {
                let stem = format!("{}", id.0 as u32);
                if let Some(path) = sps_index(self).get(&stem)
                    && let Ok(rest) = std::fs::read(path)
                {
                    data.extend_from_slice(&rest);
                }
            }
        } else {
            let path: String = h
                .get(0x28..)
                .unwrap_or(&[])
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| if b == b'\\' { '/' } else { b as char })
                .collect();
            let file = if path.is_empty() {
                sps_index(self).get(&format!("{}", id.0 as u32)).cloned()
            } else {
                self.game().path(&path)
            };
            let file = file.ok_or(Error::Missing(id))?;
            data = std::fs::read(&file).map_err(|_| Error::Missing(id))?;
        }
        let looping = mw_audio::sps::header(&data).is_ok_and(|(h, _)| h.looping);
        let pcm = mw_audio::sps::decode(&data).map_err(|_| Error::Missing(id))?;
        Ok((pcm, gain, looping))
    }
}

/// The stream already holds its end (`E`) block.
fn ends_with_end_block(data: &[u8]) -> bool {
    let mut o = 0;
    let mut last = 0u8;
    while o + 4 <= data.len() {
        let size = u32::from_be_bytes([0, data[o + 1], data[o + 2], data[o + 3]]) as usize;
        if size < 4 {
            break;
        }
        last = data[o];
        o += size;
    }
    last == b'E'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_stop_at_padding() {
        let mut d = vec![b'H', 0, 0, 8, 1, 2, 3, 4, b'D', 0, 0, 5, 9];
        d.extend_from_slice(&[0; 7]);
        assert_eq!(blocks_len(&d), 13);
        assert!(!ends_with_end_block(&d[..13]));
        d.truncate(13);
        d.extend_from_slice(&[b'E', 0, 0, 4]);
        assert!(ends_with_end_block(&d));
    }
}

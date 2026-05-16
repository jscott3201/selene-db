//! BRIEF-109 named-index snapshot wrapper.

use std::collections::HashSet;
use std::sync::Arc;

use selene_graph::SubTag;

use crate::registry::{
    decode_hnsw_config, decode_ivf_config, encode_hnsw_config, encode_ivf_config,
};
use crate::{HnswConfig, IvfConfig, VectorError};

use super::{decode_failed, encode_failed};

const SNAPSHOT_WRAPPER_VERSION: u16 = 1;

/// One decoded HNSW snapshot wrapper entry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HnswSnapshotEntry {
    pub(crate) name: Arc<str>,
    pub(crate) config: HnswConfig,
    pub(crate) section: Vec<u8>,
}

/// One decoded IVF snapshot wrapper entry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IvfSnapshotEntry {
    pub(crate) name: Arc<str>,
    pub(crate) config: IvfConfig,
    pub(crate) section: Vec<u8>,
}

pub(crate) fn encode_hnsw_entries(
    sub_tag: SubTag,
    entries: Vec<HnswSnapshotEntry>,
) -> Result<Vec<u8>, VectorError> {
    encode_entries(
        sub_tag,
        entries.into_iter().map(|entry| {
            encode_hnsw_config(&entry.config).map(|config| RawEntry {
                name: entry.name,
                config,
                section: entry.section,
            })
        }),
    )
}

pub(crate) fn encode_ivf_entries(
    sub_tag: SubTag,
    entries: Vec<IvfSnapshotEntry>,
) -> Result<Vec<u8>, VectorError> {
    encode_entries(
        sub_tag,
        entries.into_iter().map(|entry| {
            encode_ivf_config(&entry.config).map(|config| RawEntry {
                name: entry.name,
                config,
                section: entry.section,
            })
        }),
    )
}

pub(crate) fn decode_hnsw_entries(
    sub_tag: SubTag,
    bytes: &[u8],
    default_config: HnswConfig,
) -> Result<Vec<HnswSnapshotEntry>, VectorError> {
    if !is_v1_wrapper(bytes) {
        return Ok(vec![HnswSnapshotEntry {
            name: Arc::from("default"),
            config: default_config,
            section: bytes.to_vec(),
        }]);
    }
    decode_entries(sub_tag, bytes, |config| {
        decode_hnsw_config(config).map_err(|error| {
            decode_failed(
                sub_tag,
                format!("HNSW config metadata decode failed: {error}"),
            )
        })
    })
    .map(|entries| {
        entries
            .into_iter()
            .map(|entry| HnswSnapshotEntry {
                name: entry.name,
                config: entry.config,
                section: entry.section,
            })
            .collect()
    })
}

pub(crate) fn decode_ivf_entries(
    sub_tag: SubTag,
    bytes: &[u8],
    default_config: IvfConfig,
) -> Result<Vec<IvfSnapshotEntry>, VectorError> {
    if !is_v1_wrapper(bytes) {
        return Ok(vec![IvfSnapshotEntry {
            name: Arc::from("default"),
            config: default_config,
            section: bytes.to_vec(),
        }]);
    }
    decode_entries(sub_tag, bytes, |config| {
        decode_ivf_config(config).map_err(|error| {
            decode_failed(
                sub_tag,
                format!("IVF config metadata decode failed: {error}"),
            )
        })
    })
    .map(|entries| {
        entries
            .into_iter()
            .map(|entry| IvfSnapshotEntry {
                name: entry.name,
                config: entry.config,
                section: entry.section,
            })
            .collect()
    })
}

fn is_v1_wrapper(bytes: &[u8]) -> bool {
    matches!(bytes.get(0..2), Some([0x01, 0x00]))
}

struct RawEntry {
    name: Arc<str>,
    config: Vec<u8>,
    section: Vec<u8>,
}

struct DecodedEntry<C> {
    name: Arc<str>,
    config: C,
    section: Vec<u8>,
}

fn encode_entries(
    sub_tag: SubTag,
    entries: impl Iterator<Item = Result<RawEntry, VectorError>>,
) -> Result<Vec<u8>, VectorError> {
    let entries = entries
        .map(|entry| {
            entry.map_err(|error| {
                encode_failed(sub_tag, format!("config metadata encode failed: {error}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let entry_count = u32::try_from(entries.len())
        .map_err(|_| encode_failed(sub_tag, "snapshot wrapper entry count exceeds u32"))?;
    if entry_count == 0 {
        return Err(encode_failed(
            sub_tag,
            "snapshot wrapper must contain at least one entry",
        ));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&SNAPSHOT_WRAPPER_VERSION.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    for entry in entries {
        let name_len = u16::try_from(entry.name.len())
            .map_err(|_| encode_failed(sub_tag, "snapshot wrapper name exceeds u16"))?;
        let config_len = u32::try_from(entry.config.len())
            .map_err(|_| encode_failed(sub_tag, "snapshot wrapper config exceeds u32"))?;
        let section_len = u32::try_from(entry.section.len())
            .map_err(|_| encode_failed(sub_tag, "snapshot wrapper section exceeds u32"))?;
        if name_len == 0 {
            return Err(encode_failed(
                sub_tag,
                "snapshot wrapper name must be non-empty",
            ));
        }
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(entry.name.as_bytes());
        out.extend_from_slice(&config_len.to_le_bytes());
        out.extend_from_slice(&entry.config);
        out.extend_from_slice(&section_len.to_le_bytes());
        out.extend_from_slice(&entry.section);
    }
    Ok(out)
}

fn decode_entries<C>(
    sub_tag: SubTag,
    bytes: &[u8],
    decode_config: impl Fn(&[u8]) -> Result<C, VectorError>,
) -> Result<Vec<DecodedEntry<C>>, VectorError> {
    let mut input = bytes;
    let version = read_u16(sub_tag, &mut input, "wrapper version")?;
    if version != SNAPSHOT_WRAPPER_VERSION {
        return Err(decode_failed(
            sub_tag,
            format!("unsupported snapshot wrapper version {version}"),
        ));
    }
    let entry_count = read_u32(sub_tag, &mut input, "wrapper entry count")? as usize;
    if entry_count == 0 {
        return Err(decode_failed(
            sub_tag,
            "snapshot wrapper must contain at least one entry",
        ));
    }
    let mut seen = HashSet::with_capacity(entry_count);
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let name_len = read_u16(sub_tag, &mut input, "wrapper name length")? as usize;
        let name = read_str(sub_tag, &mut input, name_len, "wrapper name")?;
        if !seen.insert(name.to_owned()) {
            return Err(decode_failed(
                sub_tag,
                format!("duplicate snapshot wrapper entry '{name}'"),
            ));
        }
        let config_len = read_u32(sub_tag, &mut input, "wrapper config length")? as usize;
        let config_bytes = read_bytes(sub_tag, &mut input, config_len, "wrapper config")?;
        let config = decode_config(config_bytes)?;
        let section_len = read_u32(sub_tag, &mut input, "wrapper section length")? as usize;
        let section = read_bytes(sub_tag, &mut input, section_len, "wrapper section")?.to_vec();
        entries.push(DecodedEntry {
            name: Arc::from(name),
            config,
            section,
        });
    }
    if !input.is_empty() {
        return Err(decode_failed(
            sub_tag,
            "snapshot wrapper has trailing bytes",
        ));
    }
    Ok(entries)
}

fn read_u16(sub_tag: SubTag, bytes: &mut &[u8], label: &'static str) -> Result<u16, VectorError> {
    let raw = read_bytes(sub_tag, bytes, 2, label)?;
    Ok(u16::from_le_bytes(
        raw.try_into().expect("slice length checked for u16"),
    ))
}

fn read_u32(sub_tag: SubTag, bytes: &mut &[u8], label: &'static str) -> Result<u32, VectorError> {
    let raw = read_bytes(sub_tag, bytes, 4, label)?;
    Ok(u32::from_le_bytes(
        raw.try_into().expect("slice length checked for u32"),
    ))
}

fn read_bytes<'a>(
    sub_tag: SubTag,
    bytes: &mut &'a [u8],
    len: usize,
    label: &'static str,
) -> Result<&'a [u8], VectorError> {
    let Some((raw, rest)) = bytes.split_at_checked(len) else {
        return Err(decode_failed(sub_tag, format!("{label} truncated")));
    };
    *bytes = rest;
    Ok(raw)
}

fn read_str<'a>(
    sub_tag: SubTag,
    bytes: &mut &'a [u8],
    len: usize,
    label: &'static str,
) -> Result<&'a str, VectorError> {
    let raw = read_bytes(sub_tag, bytes, len, label)?;
    if raw.is_empty() {
        return Err(decode_failed(sub_tag, format!("{label} must be non-empty")));
    }
    std::str::from_utf8(raw)
        .map_err(|error| decode_failed(sub_tag, format!("{label} is not UTF-8: {error}")))
}

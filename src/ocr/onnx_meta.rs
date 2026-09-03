//! Read ONNX `metadata_props` without committing an ORT session.
//!
//! ModelProto.graph is skipped by seek. The walker never loads tensors
//! or initializes ORT.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::{metadata_mismatch, PackMetadata};

const WIRE_VARINT: u32 = 0;
const WIRE_I64: u32 = 1;
const WIRE_LEN: u32 = 2;
const WIRE_I32: u32 = 5;
const FIELD_METADATA_PROPS: u32 = 14;

pub(super) fn inspect_onnx_files(files: &[(&Path, &str)]) -> PackMetadata {
    let mut stamps = Vec::with_capacity(files.len());
    for (path, expected) in files {
        match read_onnx_metadata(path) {
            Ok(map) => stamps.push((stamp_from_map(&map), *expected)),
            Err(_) => return metadata_mismatch("ONNX metadata is unreadable"),
        }
    }
    super::inspect_stamps(&stamps)
}

pub(super) fn stamp_from_map(map: &BTreeMap<String, String>) -> super::FileStamp {
    super::FileStamp {
        schema: map.get("localtex.metadata_schema").cloned(),
        pack_id: map.get("localtex.pack_id").cloned(),
        component: map.get("localtex.component").cloned(),
    }
}

pub(super) fn read_onnx_metadata(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut props = BTreeMap::new();
    while let Some((field, wire)) = read_tag(&mut file)? {
        match (field, wire) {
            (FIELD_METADATA_PROPS, WIRE_LEN) => {
                let entry = read_len_bytes(&mut file)?;
                if let Some((key, value)) = parse_string_entry(&entry)? {
                    props.insert(key, value);
                }
            }
            (_, WIRE_VARINT) => {
                read_varint(&mut file)?;
            }
            (_, WIRE_I64) => skip_n(&mut file, 8)?,
            (_, WIRE_LEN) => skip_len_field(&mut file)?,
            (_, WIRE_I32) => skip_n(&mut file, 4)?,
            _ => bail!("unsupported protobuf wire type {wire}"),
        }
    }
    Ok(props)
}

fn read_tag<R: Read>(reader: &mut R) -> Result<Option<(u32, u32)>> {
    match try_read_varint(reader)? {
        None => Ok(None),
        Some(tag) => {
            let field = (tag >> 3) as u32;
            let wire = (tag & 7) as u32;
            Ok(Some((field, wire)))
        }
    }
}

fn try_read_varint<R: Read>(reader: &mut R) -> Result<Option<u64>> {
    let mut byte = [0u8; 1];
    match reader.read(&mut byte) {
        Ok(0) => Ok(None),
        Ok(_) => {
            if byte[0] < 0x80 {
                return Ok(Some(u64::from(byte[0])));
            }
            let mut value = u64::from(byte[0] & 0x7f);
            let mut shift = 7;
            for _ in 0..9 {
                reader.read_exact(&mut byte)?;
                value |= u64::from(byte[0] & 0x7f) << shift;
                if byte[0] < 0x80 {
                    return Ok(Some(value));
                }
                shift += 7;
            }
            bail!("protobuf varint is too long")
        }
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn read_varint<R: Read>(reader: &mut R) -> Result<u64> {
    try_read_varint(reader)?.context("truncated protobuf varint")
}

fn read_len_bytes<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let len = read_varint(reader)? as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

fn skip_len_field<R: Read + Seek>(reader: &mut R) -> Result<()> {
    let len = read_varint(reader)? as i64;
    reader
        .seek(SeekFrom::Current(len))
        .context("seek past protobuf field")?;
    Ok(())
}

fn skip_n<R: Read>(reader: &mut R, n: u64) -> Result<()> {
    let mut buf = [0u8; 8];
    let n = n as usize;
    reader.read_exact(&mut buf[..n])?;
    Ok(())
}

fn parse_string_entry(bytes: &[u8]) -> Result<Option<(String, String)>> {
    let mut cur = io::Cursor::new(bytes);
    let mut key = None;
    let mut value = None;
    while let Some((field, wire)) = read_tag(&mut cur)? {
        match (field, wire) {
            (1, WIRE_LEN) => key = Some(read_len_string(&mut cur)?),
            (2, WIRE_LEN) => value = Some(read_len_string(&mut cur)?),
            (_, WIRE_VARINT) => {
                read_varint(&mut cur)?;
            }
            (_, WIRE_I64) => skip_n(&mut cur, 8)?,
            (_, WIRE_LEN) => {
                let n = read_varint(&mut cur)? as u64;
                skip_n_cursor(&mut cur, n)?;
            }
            (_, WIRE_I32) => skip_n(&mut cur, 4)?,
            _ => bail!("unsupported protobuf wire type {wire} in metadata entry"),
        }
    }
    Ok(match (key, value) {
        (Some(key), Some(value)) => Some((key, value)),
        _ => None,
    })
}

fn read_len_string<R: Read>(reader: &mut R) -> Result<String> {
    Ok(String::from_utf8(read_len_bytes(reader)?)?)
}

fn skip_n_cursor(cur: &mut io::Cursor<&[u8]>, n: u64) -> Result<()> {
    let pos = cur.position().saturating_add(n);
    if pos > cur.get_ref().len() as u64 {
        bail!("truncated protobuf field");
    }
    cur.set_position(pos);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn write_len_field(buf: &mut Vec<u8>, field: u32, payload: &[u8]) {
        write_varint(buf, u64::from((field << 3) | WIRE_LEN));
        write_varint(buf, payload.len() as u64);
        buf.extend_from_slice(payload);
    }

    fn string_entry(key: &str, value: &str) -> Vec<u8> {
        let mut entry = Vec::new();
        write_len_field(&mut entry, 1, key.as_bytes());
        write_len_field(&mut entry, 2, value.as_bytes());
        entry
    }

    #[test]
    fn walker_skips_graph_and_reads_props() {
        let mut graph = vec![0u8; 256 * 1024];
        let last = graph.len() - 1;
        graph[0] = 0xab;
        graph[last] = 0xcd;
        let mut model = Vec::new();
        write_len_field(&mut model, 7, &graph);
        write_len_field(
            &mut model,
            FIELD_METADATA_PROPS,
            &string_entry("localtex.pack_id", "open-v2"),
        );
        write_len_field(
            &mut model,
            FIELD_METADATA_PROPS,
            &string_entry("localtex.component", "layout"),
        );

        let path =
            std::env::temp_dir().join(format!("localtex-onnx-meta-{}.onnx", std::process::id()));
        File::create(&path).unwrap().write_all(&model).unwrap();
        let map = read_onnx_metadata(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(
            map.get("localtex.pack_id").map(String::as_str),
            Some("open-v2")
        );
        assert_eq!(
            map.get("localtex.component").map(String::as_str),
            Some("layout")
        );
    }

    #[test]
    fn empty_props_are_legal() {
        let path =
            std::env::temp_dir().join(format!("localtex-onnx-empty-{}.onnx", std::process::id()));
        std::fs::write(&path, []).unwrap();
        let map = read_onnx_metadata(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(map.is_empty());
    }
}

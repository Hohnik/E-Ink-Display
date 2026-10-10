//! Lossless LZ compression in independent groups of four 64×64 glyphs.
//! Glyph lookup uses a fixed 2 KB stack buffer and never allocates on the device.
const RAW_RECORD: usize = 514;
pub const PIXELS: usize = 512;
const GROUP: usize = 4;
const HEADER: usize = 8;

#[allow(dead_code)]
pub fn pack(raw: &[u8]) -> Vec<u8> {
    use std::collections::HashMap;
    assert!(raw.len().is_multiple_of(RAW_RECORD));
    let count = raw.len() / RAW_RECORD;
    assert!(count <= u16::MAX as usize);
    let groups = count.div_ceil(GROUP);
    let mut packed = Vec::new();
    packed.extend_from_slice(b"JPF2");
    packed.extend_from_slice(&(count as u16).to_le_bytes());
    packed.extend_from_slice(&(GROUP as u16).to_le_bytes());
    let mut previous = None;
    for glyph in raw.as_chunks::<RAW_RECORD>().0 {
        let code = u16::from_be_bytes([glyph[0], glyph[1]]);
        assert!(previous.is_none_or(|p| p < code));
        previous = Some(code);
        packed.extend_from_slice(&glyph[..2]);
    }
    let offsets = packed.len();
    packed.resize(offsets + (groups + 1) * 4, 0);
    for (group, records) in raw.chunks(GROUP * RAW_RECORD).enumerate() {
        let offset = packed.len() as u32;
        packed[offsets + group * 4..offsets + (group + 1) * 4]
            .copy_from_slice(&offset.to_le_bytes());
        let pixels: Vec<_> = records
            .as_chunks::<RAW_RECORD>()
            .0
            .iter()
            .flat_map(|r| r[2..].iter().copied())
            .collect();
        let mut positions: HashMap<[u8; 3], Vec<usize>> = HashMap::new();
        let mut at = 0;
        while at < pixels.len() {
            let control_at = packed.len();
            packed.push(0);
            for bit in 0..8 {
                if at == pixels.len() {
                    break;
                }
                let mut best = (0usize, 0usize);
                let limit = (pixels.len() - at).min(34);
                if limit >= 3 {
                    let key = [pixels[at], pixels[at + 1], pixels[at + 2]];
                    if let Some(candidates) = positions.get(&key) {
                        for &start in candidates.iter().rev() {
                            let mut len = 3;
                            while len < limit && pixels[start + len] == pixels[at + len] {
                                len += 1;
                            }
                            if len > best.1 {
                                best = (at - start, len);
                            }
                            if len == limit {
                                break;
                            }
                        }
                    }
                }
                let consumed = if best.1 >= 3 {
                    packed[control_at] |= 1 << bit;
                    let token = (((best.0 - 1) << 5) | (best.1 - 3)) as u16;
                    packed.extend_from_slice(&token.to_le_bytes());
                    best.1
                } else {
                    packed.push(pixels[at]);
                    1
                };
                for pos in at..at + consumed {
                    if pos + 2 < pixels.len() {
                        positions
                            .entry([pixels[pos], pixels[pos + 1], pixels[pos + 2]])
                            .or_default()
                            .push(pos);
                    }
                }
                at += consumed;
            }
        }
    }
    let end = packed.len() as u32;
    packed[offsets + groups * 4..offsets + (groups + 1) * 4].copy_from_slice(&end.to_le_bytes());
    packed
}

#[allow(dead_code)]
pub fn glyph(atlas: &[u8], code: u16) -> Option<[u8; PIXELS]> {
    let compressed = atlas.starts_with(b"JPF2");
    let (count, base, stride) = if compressed {
        if u16::from_le_bytes(atlas.get(6..8)?.try_into().ok()?) as usize != GROUP {
            return None;
        }
        (
            u16::from_le_bytes(atlas.get(4..6)?.try_into().ok()?) as usize,
            HEADER,
            2,
        )
    } else {
        if !atlas.len().is_multiple_of(RAW_RECORD) {
            return None;
        }
        (atlas.len() / RAW_RECORD, 0, RAW_RECORD)
    };
    atlas.get(..base + count * stride)?;
    let mut low = 0;
    let mut high = count;
    while low < high {
        let mid = (low + high) / 2;
        let at = base + mid * stride;
        let found = u16::from_be_bytes([atlas[at], atlas[at + 1]]);
        if found < code {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low == count {
        return None;
    }
    let at = base + low * stride;
    if u16::from_be_bytes([atlas[at], atlas[at + 1]]) != code {
        return None;
    }
    let mut out = [0u8; PIXELS];
    if !compressed {
        out.copy_from_slice(atlas.get(at + 2..at + RAW_RECORD)?);
        return Some(out);
    }
    let group = low / GROUP;
    let offsets = HEADER + count * 2;
    let offset = |index: usize| -> Option<usize> {
        let pos = offsets + index * 4;
        Some(u32::from_le_bytes(atlas.get(pos..pos + 4)?.try_into().ok()?) as usize)
    };
    let start = offset(group)?;
    let end = offset(group + 1)?;
    if start < offsets + (count.div_ceil(GROUP) + 1) * 4 {
        return None;
    }
    let payload = atlas.get(start..end)?;
    let mut decoded = [0u8; GROUP * PIXELS];
    let expected = (count - group * GROUP).min(GROUP) * PIXELS;
    let mut input = 0;
    let mut output = 0;
    while output < expected {
        let control = *payload.get(input)?;
        input += 1;
        for bit in 0..8 {
            if output == expected {
                break;
            }
            if control & (1 << bit) == 0 {
                decoded[output] = *payload.get(input)?;
                input += 1;
                output += 1;
            } else {
                let token =
                    u16::from_le_bytes(payload.get(input..input + 2)?.try_into().ok()?) as usize;
                input += 2;
                let distance = (token >> 5) + 1;
                let len = (token & 31) + 3;
                if distance > output || output + len > expected {
                    return None;
                }
                for _ in 0..len {
                    decoded[output] = decoded[output - distance];
                    output += 1;
                }
            }
        }
    }
    if input != payload.len() {
        return None;
    }
    let at = low % GROUP * PIXELS;
    out.copy_from_slice(&decoded[at..at + PIXELS]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compression_preserves_every_japanese_glyph() {
        let raw = include_bytes!("../japanese_font.bin");
        let packed = pack(raw);
        println!("Font: {} -> {} bytes", raw.len(), packed.len());
        assert!(packed.len() < raw.len() * 3 / 4);
        for record in raw.as_chunks::<RAW_RECORD>().0 {
            let code = u16::from_be_bytes([record[0], record[1]]);
            assert_eq!(glyph(&packed, code).unwrap().as_slice(), &record[2..]);
        }
        assert_eq!(glyph(&packed, 0), None);
    }
    #[test]
    fn corrupt_font_is_rejected_without_panicking() {
        for atlas in [b"JPF2".as_slice(), b"JPF2\x01\x00".as_slice()] {
            assert_eq!(glyph(atlas, 1), None);
        }
        let mut record = vec![0xff; RAW_RECORD];
        record[..2].copy_from_slice(&1u16.to_be_bytes());
        let mut packed = pack(&record);
        packed.truncate(packed.len() - 1);
        assert_eq!(glyph(&packed, 1), None);
        let packed = pack(&record);
        for len in 0..packed.len() {
            assert_eq!(glyph(&packed[..len], 1), None);
        }
    }
}

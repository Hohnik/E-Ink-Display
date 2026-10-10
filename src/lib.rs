//! Allocation-bounded Japanese deck handling and 1-bit display rendering.
pub mod font;
pub mod panel;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const WIDTH: usize = 800;
pub const HEIGHT: usize = 480;
pub const FRAME_BYTES: usize = WIDTH * HEIGHT / 8;
pub const MAX_CARDS: usize = 100;
pub const MAX_DECK_BYTES: u64 = 128 * 1024;
pub const MAX_CARD_BYTES: usize = 4096;
pub const GLYPH_SIZE: usize = 32;
pub const GLYPH_SOURCE_SIZE: usize = 64;
pub const GLYPH_SOURCE_ROW_BYTES: usize = 8;
pub const GLYPH_RECORD_BYTES: usize = 514;
pub const GLYPH_COUNT: usize = 6942;

const LATIN_FONT: &[u8] = include_bytes!("../latin_font.bin");
const LATIN_FIRST: u8 = 32;
const LATIN_LAST: u8 = 126;
const LATIN_GLYPHS: usize = (LATIN_LAST - LATIN_FIRST + 1) as usize;
const LATIN_SMALL_WIDTH: usize = 8;
const LATIN_SMALL_HEIGHT: usize = 16;
const LATIN_SMALL_BYTES: usize = LATIN_SMALL_HEIGHT;
const LATIN_LARGE_WIDTH: usize = 16;
const LATIN_LARGE_HEIGHT: usize = 32;
const LATIN_LARGE_BYTES: usize = LATIN_LARGE_WIDTH / 8 * LATIN_LARGE_HEIGHT;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Card {
    pub id: String,
    pub kind: String,
    pub japanese: String,
    pub reading: String,
    pub meaning: String,
    pub example: Example,
    #[serde(default)]
    pub kanji: Vec<KanjiMeaning>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub ja: String,
    pub en: String,
    #[serde(default)]
    pub words: Vec<GlossedWord>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GlossedWord {
    pub japanese: String,
    pub reading: String,
    pub english: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KanjiMeaning {
    pub kanji: String,
    pub english: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardOffset {
    pub start: u32,
    pub len: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckIndex {
    pub date: String,
    pub cards: Vec<CardOffset>,
}

fn valid_date(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn validate_card(card: &Card, ids: &mut HashSet<String>) -> io::Result<()> {
    let fields = [
        (&card.id, 32),
        (&card.kind, 24),
        (&card.japanese, 64),
        (&card.reading, 64),
        (&card.meaning, 80),
        (&card.example.ja, 120),
        (&card.example.en, 120),
    ];
    if fields
        .iter()
        .any(|(s, max)| s.trim().is_empty() || s.len() > *max)
        || card.example.words.len() > 16
        || card.kanji.len() > 16
        || card.example.words.iter().any(|word| {
            word.japanese.trim().is_empty()
                || word.japanese.len() > 48
                || word.reading.len() > 32
                || word.english.trim().is_empty()
                || word.english.len() > 40
        })
        || card.kanji.iter().any(|entry| {
            entry.kanji.chars().count() != 1
                || entry.english.trim().is_empty()
                || entry.english.len() > 32
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "card text empty, too long, or has invalid glosses",
        ));
    }
    if !ids.insert(card.id.clone()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "duplicate card id",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeckHeader {
    date: String,
    #[serde(deserialize_with = "count_cards")]
    cards: usize,
}

fn count_cards<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    struct Cards;
    impl<'de> serde::de::Visitor<'de> for Cards {
        type Value = usize;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a non-empty cards array")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<usize, A::Error> {
            let mut count = 0;
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                count += 1;
                if count > MAX_CARDS {
                    return Err(serde::de::Error::custom("too many cards"));
                }
            }
            if count == 0 {
                return Err(serde::de::Error::custom("empty cards array"));
            }
            Ok(count)
        }
    }
    deserializer.deserialize_seq(Cards)
}

/// Scan one card object at a time; only offsets and the current card are held in RAM.
pub fn index_deck(path: impl AsRef<Path>) -> io::Result<DeckIndex> {
    let path = path.as_ref();
    let size = std::fs::metadata(path)?.len();
    if size == 0 || size > MAX_DECK_BYTES {
        return Err(invalid("deck must be 1..131072 bytes"));
    }
    // Validate the entire envelope with a streaming parser. IgnoredAny skips
    // card values without retaining them; the bounded scanner below validates
    // each card and records its exact file offset.
    let reader = io::BufReader::with_capacity(512, File::open(path)?);
    let header: DeckHeader = serde_json::from_reader(reader)
        .map_err(|_| invalid("invalid deck JSON: expected date and cards"))?;
    if !valid_date(&header.date) {
        return Err(invalid("date must be YYYY-MM-DD"));
    }
    let date = header.date;
    let mut file = File::open(path)?;
    let marker = b"\"cards\"";
    let mut carry = Vec::new();
    let mut chunk = [0u8; 512];
    let mut data = Vec::with_capacity(512 + marker.len());
    let mut file_pos = 0u64;
    let mut found = false;
    let mut in_array = false;
    let mut card_start: Option<u64> = None;
    let mut card = Vec::with_capacity(MAX_CARD_BYTES.min(512));
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut offsets = Vec::with_capacity(MAX_CARDS);
    let mut ids = HashSet::with_capacity(MAX_CARDS);

    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        data.clear();
        data.extend_from_slice(&carry);
        data.extend_from_slice(&chunk[..n]);
        let data_base = file_pos.saturating_sub(carry.len() as u64);
        let scan_from = if found {
            0
        } else if let Some(i) = find_subslice(&data, marker) {
            found = true;
            i + marker.len()
        } else {
            let keep = marker.len().saturating_sub(1).min(data.len());
            carry.clear();
            carry.extend_from_slice(&data[data.len() - keep..]);
            file_pos += n as u64;
            continue;
        };
        carry.clear();
        for (i, &b) in data.iter().enumerate().skip(scan_from) {
            let absolute = data_base + i as u64;
            if !in_array {
                if b == b'[' {
                    in_array = true;
                }
                continue;
            }
            if card_start.is_none() {
                if b == b']' {
                    break;
                }
                if b == b'{' {
                    card_start = Some(absolute);
                    card.clear();
                    card.push(b);
                    depth = 1;
                    in_string = false;
                    escaped = false;
                }
                continue;
            }
            card.push(b);
            if card.len() > MAX_CARD_BYTES {
                return Err(invalid("card is larger than 4096 bytes"));
            }
            if in_string {
                if b == b'"' && !escaped {
                    in_string = false;
                }
                escaped = b == b'\\' && !escaped;
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        let card_value: Card = serde_json::from_slice(&card)
                            .map_err(|_| invalid("invalid card JSON or card fields"))?;
                        validate_card(&card_value, &mut ids)?;
                        let start = card_start.take().unwrap();
                        let len = card.len();
                        if start > u32::MAX as u64 || len > u16::MAX as usize {
                            return Err(invalid("deck index overflow"));
                        }
                        offsets.push(CardOffset {
                            start: start as u32,
                            len: len as u16,
                        });
                        if offsets.len() > MAX_CARDS {
                            return Err(invalid("too many cards"));
                        }
                    }
                }
                _ => {}
            }
        }
        file_pos += n as u64;
    }
    if !found || !in_array || card_start.is_some() || offsets.len() != header.cards {
        return Err(invalid("expected a non-empty cards array"));
    }
    Ok(DeckIndex {
        date,
        cards: offsets,
    })
}

pub fn read_card(path: impl AsRef<Path>, offset: CardOffset) -> io::Result<Card> {
    if offset.len == 0
        || offset.len as usize > MAX_CARD_BYTES
        || offset.start as u64 + offset.len as u64 > MAX_DECK_BYTES
    {
        return Err(invalid("invalid card offset"));
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset.start as u64))?;
    let mut bytes = vec![0u8; offset.len as usize];
    file.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| invalid("stored card is invalid"))
}

pub fn save_index(path: impl AsRef<Path>, index: &DeckIndex) -> io::Result<()> {
    let mut out = File::create(path)?;
    for entry in &index.cards {
        out.write_all(&entry.start.to_le_bytes())?;
        out.write_all(&entry.len.to_le_bytes())?;
    }
    out.sync_all()
}

pub fn load_index(path: impl AsRef<Path>, date: String, count: usize) -> io::Result<DeckIndex> {
    if !valid_date(&date) || count == 0 || count > MAX_CARDS {
        return Err(invalid("invalid stored deck metadata"));
    }
    let mut file = File::open(path)?;
    if file.metadata()?.len() != (count * 6) as u64 {
        return Err(invalid("invalid stored index size"));
    }
    let mut bytes = vec![0; count * 6];
    file.read_exact(&mut bytes)?;
    let cards: Vec<_> = bytes
        .as_chunks::<6>()
        .0
        .iter()
        .map(|b| CardOffset {
            start: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            len: u16::from_le_bytes([b[4], b[5]]),
        })
        .collect();
    let mut end = 0usize;
    for entry in &cards {
        let start = entry.start as usize;
        let len = entry.len as usize;
        if len == 0 || len > MAX_CARD_BYTES || start < end || start + len > MAX_DECK_BYTES as usize
        {
            return Err(invalid("invalid stored card offset"));
        }
        end = start + len;
    }
    Ok(DeckIndex { date, cards })
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Render a glanceable card with readings above Japanese and direct glosses below it.
pub fn render_card(
    frame: &mut [u8],
    atlas: &[u8],
    card: &Card,
    date: &str,
    number: usize,
    total: usize,
) -> io::Result<()> {
    if frame.len() != FRAME_BYTES {
        return Err(invalid("framebuffer must be exactly 48000 bytes"));
    }
    frame.fill(0xff);
    draw_text(frame, atlas, &format!("JAPANESE  {date}"), 24, 4, 1, 600, 1)?;
    let progress = format!("{number}/{total}");
    let progress_width = text_width(&progress, 1);
    draw_text(
        frame,
        atlas,
        &progress,
        776usize.saturating_sub(progress_width),
        4,
        1,
        progress_width,
        1,
    )?;

    let large_gloss_lines = gloss_line_count(&card.example.words, true);
    let divider_y = if large_gloss_lines > 1 {
        draw_card_heading_compact(frame, atlas, card)?
    } else {
        draw_card_heading_centered(frame, atlas, card)?
    };
    horizontal(frame, 32, divider_y, 736);

    let mut y = divider_y + 8;
    if card.example.words.is_empty() {
        let sentence_scale = fit_scale(&card.example.ja, 704, 2);
        draw_centered(frame, atlas, &card.example.ja, y, sentence_scale)?;
        y += GLYPH_SIZE * sentence_scale + 8;
    } else {
        y = draw_glossed_words(frame, atlas, &card.example.words, y, large_gloss_lines <= 2)?;
    }

    let natural = format!("Natural: {}", card.example.en);
    let natural_scale = fit_scale(&natural, 704, 2);
    draw_centered(frame, atlas, &natural, y + 6, natural_scale)?;
    y += LATIN_SMALL_HEIGHT * natural_scale + 12;
    if !card.kanji.is_empty() && y < HEIGHT {
        draw_kanji_meanings(frame, atlas, &card.kanji, y)?;
    }
    Ok(())
}

fn text_width(text: &str, scale: usize) -> usize {
    text.chars()
        .map(|c| (if c.is_ascii() { 8 } else { 32 }) * scale)
        .sum()
}
fn fit_scale(text: &str, max_width: usize, maximum: usize) -> usize {
    (max_width / text_width(text, 1).max(1)).clamp(1, maximum)
}

fn draw_centered(
    frame: &mut [u8],
    atlas: &[u8],
    text: &str,
    y: usize,
    scale: usize,
) -> io::Result<()> {
    let width = text_width(text, scale);
    draw_text(
        frame,
        atlas,
        text,
        (WIDTH.saturating_sub(width)) / 2,
        y,
        scale,
        width,
        1,
    )
}

fn gloss_width(word: &GlossedWord, large: bool) -> usize {
    let japanese_scale = if large { 2 } else { 1 };
    let english_scale = if large { 2 } else { 1 };
    text_width(&word.reading, 1)
        .max(text_width(&word.japanese, japanese_scale))
        .max(text_width(&word.english, english_scale))
}

fn gloss_line_count(words: &[GlossedWord], large: bool) -> usize {
    let mut lines = usize::from(!words.is_empty());
    let mut width = 0;
    for word in words {
        let next = gloss_width(word, large);
        let candidate = width + usize::from(width > 0) * 12 + next;
        if candidate > 704 && width > 0 {
            lines += 1;
            width = next;
        } else {
            width = candidate;
        }
    }
    lines
}

fn draw_card_heading_centered(frame: &mut [u8], atlas: &[u8], card: &Card) -> io::Result<usize> {
    let japanese_scale = fit_scale(&card.japanese, 704, 2);
    draw_centered(frame, atlas, &card.japanese, 24, japanese_scale)?;
    let reading_y = 24 + GLYPH_SIZE * japanese_scale + 2;
    draw_centered(frame, atlas, &card.reading, reading_y, 1)?;
    let meaning_scale = fit_scale(&card.meaning, 704, 2);
    let meaning_y = reading_y + GLYPH_SIZE + 2;
    draw_centered(frame, atlas, &card.meaning, meaning_y, meaning_scale)?;
    Ok(meaning_y + LATIN_SMALL_HEIGHT * meaning_scale + 6)
}

fn draw_card_heading_compact(frame: &mut [u8], atlas: &[u8], card: &Card) -> io::Result<usize> {
    let left_x: usize = 32;
    let left_width: usize = 336;
    let right_x: usize = 400;
    let right_width: usize = 368;
    let japanese_scale = fit_scale(&card.japanese, left_width, 2);
    let japanese_width = text_width(&card.japanese, japanese_scale);
    draw_text(
        frame,
        atlas,
        &card.japanese,
        left_x + left_width.saturating_sub(japanese_width) / 2,
        28,
        japanese_scale,
        left_width,
        1,
    )?;
    let reading_width = text_width(&card.reading, 1);
    draw_text(
        frame,
        atlas,
        &card.reading,
        right_x + right_width.saturating_sub(reading_width) / 2,
        28,
        1,
        right_width,
        1,
    )?;
    let meaning_scale = fit_scale(&card.meaning, right_width, 2);
    let meaning_width = text_width(&card.meaning, meaning_scale);
    draw_text(
        frame,
        atlas,
        &card.meaning,
        right_x + right_width.saturating_sub(meaning_width) / 2,
        64,
        meaning_scale,
        right_width,
        1,
    )?;
    Ok(100)
}

fn draw_glossed_words(
    frame: &mut [u8],
    atlas: &[u8],
    words: &[GlossedWord],
    y: usize,
    large: bool,
) -> io::Result<usize> {
    let japanese_scale = if large { 2 } else { 1 };
    let english_scale = if large { 2 } else { 1 };
    let row_height =
        GLYPH_SIZE + 2 + GLYPH_SIZE * japanese_scale + 4 + LATIN_SMALL_HEIGHT * english_scale + 4;
    let mut start = 0;
    let mut py = y;
    while start < words.len() {
        let mut end = start;
        let mut width = 0;
        while end < words.len() {
            let next = gloss_width(&words[end], large);
            let candidate = width + usize::from(end > start) * 12 + next;
            if candidate > 704 && end > start {
                break;
            }
            width = candidate;
            end += 1;
        }
        let mut x = (WIDTH.saturating_sub(width)) / 2;
        for word in &words[start..end] {
            let column = gloss_width(word, large);
            let reading_width = text_width(&word.reading, 1);
            draw_text(
                frame,
                atlas,
                &word.reading,
                x + column.saturating_sub(reading_width) / 2,
                py,
                1,
                column,
                1,
            )?;
            let japanese_y = py + GLYPH_SIZE + 2;
            let japanese_width = text_width(&word.japanese, japanese_scale);
            draw_text(
                frame,
                atlas,
                &word.japanese,
                x + column.saturating_sub(japanese_width) / 2,
                japanese_y,
                japanese_scale,
                column,
                1,
            )?;
            let english_y = japanese_y + GLYPH_SIZE * japanese_scale + 4;
            let english_width = text_width(&word.english, english_scale);
            draw_text(
                frame,
                atlas,
                &word.english,
                x + column.saturating_sub(english_width) / 2,
                english_y,
                english_scale,
                column,
                1,
            )?;
            x += column + 12;
        }
        start = end;
        py += row_height;
    }
    Ok(py.saturating_sub(4))
}

fn kanji_width(entry: &KanjiMeaning, scale: usize) -> usize {
    (GLYPH_SIZE * scale).max(text_width(&entry.english, scale))
}

fn draw_kanji_meanings(
    frame: &mut [u8],
    atlas: &[u8],
    meanings: &[KanjiMeaning],
    mut y: usize,
) -> io::Result<()> {
    let large_width = meanings
        .iter()
        .map(|entry| kanji_width(entry, 2))
        .sum::<usize>()
        + meanings.len().saturating_sub(1) * 12;
    let scale = if large_width <= 736 && y + 100 <= HEIGHT {
        2
    } else {
        1
    };
    let row_height = GLYPH_SIZE * scale + 2 + LATIN_SMALL_HEIGHT * scale;
    let mut start = 0;
    while start < meanings.len() && y + row_height <= HEIGHT {
        let mut end = start;
        let mut width = 0;
        while end < meanings.len() {
            let next = kanji_width(&meanings[end], scale);
            let candidate = width + usize::from(end > start) * 12 + next;
            if candidate > 736 && end > start {
                break;
            }
            width = candidate;
            end += 1;
        }
        let mut x = (WIDTH.saturating_sub(width)) / 2;
        for entry in &meanings[start..end] {
            let column = kanji_width(entry, scale);
            let kanji_width = text_width(&entry.kanji, scale);
            draw_text(
                frame,
                atlas,
                &entry.kanji,
                x + column.saturating_sub(kanji_width) / 2,
                y,
                scale,
                column,
                1,
            )?;
            let english_width = text_width(&entry.english, scale);
            draw_text(
                frame,
                atlas,
                &entry.english,
                x + column.saturating_sub(english_width) / 2,
                y + GLYPH_SIZE * scale + 2,
                scale,
                column,
                1,
            )?;
            x += column + 12;
        }
        start = end;
        y += row_height + 2;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    frame: &mut [u8],
    atlas: &[u8],
    text: &str,
    x: usize,
    y: usize,
    scale: usize,
    max_width: usize,
    max_lines: usize,
) -> io::Result<()> {
    let mut px = x;
    let mut py = y;
    let mut line = 0;
    for ch in text.chars() {
        let w = (if ch.is_ascii() { 8 } else { GLYPH_SIZE }) * scale;
        if ch == '\n' || px + w > x + max_width {
            line += 1;
            if line >= max_lines {
                break;
            }
            px = x;
            py += (GLYPH_SIZE + 2) * scale;
            if ch == '\n' {
                continue;
            }
        }
        if ch.is_ascii() {
            draw_latin_glyph(frame, ch, px, py, scale);
            px += LATIN_SMALL_WIDTH * scale;
        } else {
            if let Some(glyph) = u16::try_from(ch as u32)
                .ok()
                .and_then(|code| find_glyph(atlas, code))
            {
                draw_glyph(frame, &glyph, px, py, scale);
            }
            px += GLYPH_SIZE * scale;
        }
    }
    Ok(())
}

fn find_glyph(atlas: &[u8], code: u16) -> Option<[u8; font::PIXELS]> {
    font::glyph(atlas, code)
}

fn draw_glyph(frame: &mut [u8], glyph: &[u8], x: usize, y: usize, scale: usize) {
    if scale == 2 {
        for gy in 0..GLYPH_SOURCE_SIZE {
            for gx in 0..GLYPH_SOURCE_SIZE {
                if glyph_is_black(glyph, gx, gy) {
                    set_black(frame, x + gx, y + gy);
                }
            }
        }
        return;
    }
    for gy in 0..GLYPH_SIZE {
        for gx in 0..GLYPH_SIZE {
            let source_x = gx * 2;
            let source_y = gy * 2;
            let black = glyph_is_black(glyph, source_x, source_y) as usize
                + glyph_is_black(glyph, source_x + 1, source_y) as usize
                + glyph_is_black(glyph, source_x, source_y + 1) as usize
                + glyph_is_black(glyph, source_x + 1, source_y + 1) as usize;
            if black >= 2 {
                set_black(frame, x + gx, y + gy);
            }
        }
    }
}

fn glyph_is_black(glyph: &[u8], x: usize, y: usize) -> bool {
    glyph[y * GLYPH_SOURCE_ROW_BYTES + x / 8] & (0x80 >> (x % 8)) == 0
}

fn draw_latin_glyph(frame: &mut [u8], ch: char, x: usize, y: usize, scale: usize) {
    let code = if (LATIN_FIRST..=LATIN_LAST).contains(&(ch as u8)) {
        ch as u8
    } else {
        b'?'
    };
    let index = (code - LATIN_FIRST) as usize;
    let (glyph, width, height, row_bytes) = if scale == 1 {
        let start = index * LATIN_SMALL_BYTES;
        (
            &LATIN_FONT[start..start + LATIN_SMALL_BYTES],
            LATIN_SMALL_WIDTH,
            LATIN_SMALL_HEIGHT,
            1,
        )
    } else {
        let start = LATIN_GLYPHS * LATIN_SMALL_BYTES + index * LATIN_LARGE_BYTES;
        (
            &LATIN_FONT[start..start + LATIN_LARGE_BYTES],
            LATIN_LARGE_WIDTH,
            LATIN_LARGE_HEIGHT,
            2,
        )
    };
    for gy in 0..height {
        for gx in 0..width {
            if glyph[gy * row_bytes + gx / 8] & (0x80 >> (gx % 8)) == 0 {
                set_black(frame, x + gx, y + gy);
            }
        }
    }
}

fn horizontal(frame: &mut [u8], x: usize, y: usize, w: usize) {
    for xx in x..(x + w).min(WIDTH) {
        if y < HEIGHT {
            set_black(frame, xx, y);
        }
    }
}
fn set_black(frame: &mut [u8], x: usize, y: usize) {
    let at = y * (WIDTH / 8) + x / 8;
    frame[at] &= !(0x80 >> (x % 8));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn temp_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "eink-rust-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn sample() -> String {
        r#"{"date":"2026-10-10","cards":[{"id":"tomodachi","kind":"vocabulary","japanese":"友達","reading":"ともだち","meaning":"friend","example":{"ja":"友達と話します。","en":"I talk with a friend."}}]}"#.to_owned()
    }
    #[test]
    fn indexes_and_reads_card_without_parsing_whole_deck() {
        let p = temp_path("deck");
        let mut f = File::create(&p).unwrap();
        f.write_all(sample().as_bytes()).unwrap();
        drop(f);
        let idx = index_deck(&p).unwrap();
        assert_eq!(idx.date, "2026-10-10");
        assert_eq!(idx.cards.len(), 1);
        let card = read_card(&p, idx.cards[0]).unwrap();
        assert_eq!(card.japanese, "友達");
        let index_path = temp_path("idx");
        save_index(&index_path, &idx).unwrap();
        assert_eq!(load_index(index_path, idx.date.clone(), 1).unwrap(), idx);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn rejects_duplicate_ids_and_bad_date() {
        let p = temp_path("bad");
        let duplicate = sample().replace("]}", ",{\"id\":\"tomodachi\",\"kind\":\"vocabulary\",\"japanese\":\"友達\",\"reading\":\"ともだち\",\"meaning\":\"friend\",\"example\":{\"ja\":\"友達と話します。\",\"en\":\"I talk with a friend.\"}}]}");
        std::fs::write(&p, duplicate).unwrap();
        assert!(index_deck(&p).is_err());
        std::fs::remove_file(p).unwrap();
        let p = temp_path("bad-date");
        std::fs::write(&p, sample().replace("2026-10-10", "today")).unwrap();
        assert!(index_deck(&p).is_err());
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn indexes_and_renders_the_glossed_hundred_card_deck() {
        let path = std::path::Path::new("decks/japanese_100.json");
        let index = index_deck(path).unwrap();
        assert_eq!(index.cards.len(), 100);
        let atlas = include_bytes!("../japanese_font.bin");
        let mut frame = vec![0; FRAME_BYTES];
        for (number, offset) in index.cards.iter().copied().enumerate() {
            let card = read_card(path, offset).unwrap();
            assert!(!card.example.words.is_empty());
            render_card(
                &mut frame,
                atlas,
                &card,
                &index.date,
                number + 1,
                index.cards.len(),
            )
            .unwrap();
        }
        assert!(find_glyph(atlas, '友' as u16).is_some());
        assert!(find_glyph(atlas, 'と' as u16).is_some());
    }
    #[test]
    fn rejects_malformed_envelopes_and_trailing_data() {
        let cases = [
            sample().trim_end_matches('}').to_owned(),
            format!("{} garbage", sample()),
            sample().replace("]}", ",]}"),
            sample().replace("\"cards\":[", "\"cards\":[true,"),
            sample().replace("\"date\":", "\"unexpected\":0,\"date\":"),
        ];
        for value in cases {
            let p = temp_path("envelope");
            std::fs::write(&p, value).unwrap();
            assert!(index_deck(&p).is_err());
            std::fs::remove_file(p).unwrap();
        }
    }
    #[test]
    fn accepts_reordered_fields_and_scans_across_chunk_boundaries() {
        let value: serde_json::Value = serde_json::from_str(&sample()).unwrap();
        let p = temp_path("reordered");
        let card = serde_json::to_string(&value["cards"][0]).unwrap();
        let deck = format!(
            "{{\"cards\":[{}{}],\"date\":\"2026-10-10\"}}",
            " ".repeat(505),
            card
        );
        std::fs::write(&p, deck).unwrap();
        let index = index_deck(&p).unwrap();
        assert_eq!(index.date, "2026-10-10");
        assert_eq!(read_card(&p, index.cards[0]).unwrap().id, "tomodachi");
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn compressed_font_renders_identical_frame() {
        let value: serde_json::Value = serde_json::from_str(&sample()).unwrap();
        let card: Card = serde_json::from_value(value["cards"][0].clone()).unwrap();
        let raw = include_bytes!("../japanese_font.bin");
        let packed = font::pack(raw);
        let mut original = vec![0; FRAME_BYTES];
        let mut compressed = vec![0; FRAME_BYTES];
        render_card(&mut original, raw, &card, "2026-10-10", 1, 100).unwrap();
        render_card(&mut compressed, &packed, &card, "2026-10-10", 1, 100).unwrap();
        assert_eq!(compressed, original);
    }
    #[test]
    fn renders_exactly_one_frame() {
        let value: serde_json::Value = serde_json::from_str(&sample()).unwrap();
        let card: Card = serde_json::from_value(value["cards"][0].clone()).unwrap();
        let mut frame = vec![0u8; FRAME_BYTES];
        let atlas = include_bytes!("../japanese_font.bin");
        render_card(&mut frame, atlas, &card, "2026-10-10", 1, 1).unwrap();
        assert_eq!(frame.len(), 48000);
        assert!(frame.iter().any(|b| *b != 0xff));
    }
}

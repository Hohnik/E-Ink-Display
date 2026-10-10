use japanese_core::{FRAME_BYTES, index_deck, read_card, render_card};
use std::io::Write;

fn main() -> std::io::Result<()> {
    let number = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let output = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "card.pbm".to_owned());
    let deck = "decks/japanese_100.json";
    let index = index_deck(deck)?;
    if number == 0 || number > index.cards.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "card number is outside the deck",
        ));
    }
    let card = read_card(deck, index.cards[number - 1])?;
    let mut frame = vec![0; FRAME_BYTES];
    render_card(
        &mut frame,
        include_bytes!("../japanese_font.bin"),
        &card,
        &index.date,
        number,
        index.cards.len(),
    )?;
    let mut file = std::fs::File::create(output)?;
    file.write_all(b"P4\n800 480\n")?;
    for byte in frame {
        file.write_all(&[!byte])?;
    }
    Ok(())
}

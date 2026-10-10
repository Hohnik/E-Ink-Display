"""Build the monochrome fonts embedded in the ESP32 firmware."""
from argparse import ArgumentParser
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_JAPANESE_FONT = Path("/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc")
DEFAULT_LATIN_FONT = Path("/System/Library/Fonts/SFNSMono.ttf")
JAPANESE_SIZE = 64


def monochrome_glyph(
    char: str,
    font: ImageFont.FreeTypeFont,
    width: int,
    height: int,
    *,
    center_y: bool = True,
) -> bytes:
    scale = 2
    image = Image.new("L", (width * scale, height * scale), 255)
    draw = ImageDraw.Draw(image)
    box = draw.textbbox((0, 0), char, font=font)
    x = (width * scale - (box[2] - box[0])) // 2 - box[0]
    y = (height * scale - (box[3] - box[1])) // 2 - box[1] if center_y else 0
    draw.text((x, y), char, font=font, fill=0)
    image = image.resize((width, height), Image.Resampling.LANCZOS)
    return image.point(lambda pixel: 0 if pixel < 150 else 255).convert("1").tobytes()


def build_japanese(font_path: Path, output: Path) -> int:
    font = ImageFont.truetype(str(font_path), JAPANESE_SIZE * 2)
    records = []
    for code in range(128, 0x10000):
        char = chr(code)
        try:
            encoded = char.encode("euc_jp")
        except UnicodeEncodeError:
            continue
        if len(encoded) != 2:
            continue
        records.append(
            code.to_bytes(2, "big")
            + monochrome_glyph(char, font, JAPANESE_SIZE, JAPANESE_SIZE)
        )
    output.write_bytes(b"".join(records))
    return len(records)


def build_latin(font_path: Path, output: Path) -> int:
    characters = "".join(chr(code) for code in range(32, 127))
    small_font = ImageFont.truetype(str(font_path), 26)
    large_font = ImageFont.truetype(str(font_path), 52)
    small = b"".join(
        monochrome_glyph(char, small_font, 8, 16, center_y=False) for char in characters
    )
    large = b"".join(
        monochrome_glyph(char, large_font, 16, 32, center_y=False) for char in characters
    )
    output.write_bytes(small + large)
    return len(characters)


def main() -> None:
    parser = ArgumentParser()
    parser.add_argument("--japanese-font", type=Path, default=DEFAULT_JAPANESE_FONT)
    parser.add_argument("--latin-font", type=Path, default=DEFAULT_LATIN_FONT)
    args = parser.parse_args()
    japanese_count = build_japanese(args.japanese_font, ROOT / "japanese_font.bin")
    latin_count = build_latin(args.latin_font, ROOT / "latin_font.bin")
    print(f"Wrote {japanese_count} Japanese glyphs and {latin_count} Latin glyphs")


if __name__ == "__main__":
    main()

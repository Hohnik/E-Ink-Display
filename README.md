# Japanese Rust firmware

This is a sibling project to `eink_python`. It targets the same ESP32-WROOM-32E
and Waveshare 7.5-inch V2 display, but keeps only the Japanese deck path:

- open `POST /api/japanese/daily` accepts the Japanese JSON format (up to 100
  cards and 128 KB) and returns the saved card count;
- the deck is scanned one card at a time, then stored with a compact six-byte
  offset/length entry per card;
- Japanese, reading, meaning, and example render together; the card advances
  every minute, with a full clean refresh every three minutes;
- `GET /api/status` reports total 8-bit free heap, largest block, minimum heap
  watermark, filesystem free bytes, and upload/render/refresh timings.

There is no browser UI, slideshow, reader, or setup/access-point mode. The
Japanese glyph atlas is compressed in the firmware image; the filesystem only
stores the uploaded deck and small card index.

`src/lib.rs` contains the deck scanner, compact index, and monochrome renderer.
`firmware/` contains the ESP-IDF runtime, Wi-Fi, HTTP routes, SPIFFS storage,
and board services. `src/panel.rs` contains the reusable Waveshare driver and
`src/font.rs` contains the lossless font codec.

## Host tests

From this directory:

```sh
cargo test --offline
```

The 13 tests cover the real 100-card deck, strict JSON envelope validation,
chunk boundaries, index persistence, reset pulse timing, waveform lengths,
BUSY read errors and missing acknowledgements, recovery after SPI errors,
reference RAM synchronization, all 6,942 compressed glyphs, and identical
framebuffer rendering with the original and compressed font.

```sh
cargo clippy --offline --all-targets -- -D warnings
```

## ESP32 build

ESP-IDF is installed at `~/esp/esp-idf` (v5.5.4), and the Rust Xtensa toolchain,
`ldproxy`, and `espflash` are installed on this machine. Set Wi-Fi credentials
in the build environment; the current runtime takes them from compile-time
environment variables:

```sh
WIFI_SSID='your-wifi' WIFI_PASSWORD='your-password' bash firmware/build.sh
```

Alternatively, put the credentials in the ignored `firmware/.env` file:

```sh
WIFI_SSID='your-wifi'
WIFI_PASSWORD='your-password'
```

The build script loads this file automatically. It uses IDF from `~/esp/esp-idf`, selects the configured Python
environment, and points the generated IDF project at the sibling's partition
table. Set the same Wi-Fi credentials each time. The custom 4 MB partition
table reserves 3 MB for the application and 960 KB for SPIFFS. Check the generated application size before flashing. The 64×64
source atlas is 3,568,188 bytes; the build script losslessly compresses it to
1,517,663 bytes. Lookup decompresses four glyphs into a fixed 2 KB stack
buffer, without allocating a font cache. The high-resolution app image is
2,633,696 bytes (83.7% of the app partition), leaving 512,032 bytes. The
filesystem still has ample room for the 128 KB maximum deck.

The MicroPython `gc.mem_free()` value and Rust's ESP-IDF heap values measure
different allocators. For a fair comparison, post the same 100-card deck, wait
for uploads to finish, and compare `heap_free_bytes`, `heap_min_free_bytes`,
and `heap_largest_block_bytes` from each `/api/status` endpoint. Compare app
image size plus filesystem free bytes when assessing flash use.

## Board test and comparison

On 2026-10-10 the Rust build connected to Wi-Fi and accepted the preserved
100-card deck. Its initial e-paper refresh failed: GPIO13 (BUSY) stayed low
after the power-on command and timed out, so the card did not appear. The API
remained available to report metrics. The original 4 MB Python flash image was
restored and verified afterward.

| Measurement | Python build on board | Rust Japanese-only build |
| --- | ---: | ---: |
| App image | 1,630,192 B | 1,992,480 B |
| Filesystem used | 1,359,872 B | 18,323 B |
| App + filesystem used | 2,990,064 B | 2,010,803 B |
| Free heap after boot with deck | 56,384 B | 151,956 B |
| Largest free heap block | Not reported | 110,592 B |

The Rust app image alone is 362,288 bytes larger (22.2%). Its app-plus-filesystem
usage is about 979 KB lower because it stores only the Japanese deck and index;
the Python filesystem also contains its runtime modules, font, image assets,
reader data, and other files. This total therefore compares the current full
Python installation with the Japanese-only Rust prototype, not equivalent
feature sets. The Python build exposes only total free heap, so its largest
free block is not directly comparable.

## Display fix and reliability improvements

The failed prototype used `FreeRtos::delay_ms(2)` for the reset-low pulse.
Its generated IDF configuration has `CONFIG_FREERTOS_HZ=100`, giving a
10 ms tick. That request cannot preserve the panel's 2 ms reset pulse.
[Waveshare's driver](https://github.com/waveshareteam/e-Paper/blob/master/RaspberryPi_JetsonNano/python/lib/waveshare_epd/epd7in5_V2.py)
uses 20 ms high, 2 ms low, 20 ms high. The repaired runtime uses
`Ets::delay_us(2000)` for the short pulse and FreeRTOS for longer waits.
BUSY polling yields at 10 ms (partial) or 20 ms (clean), retaining a 30-second
timeout without starving the idle task.

Other changes:

- Each LUT register receives the same 42 bytes as the working Python driver
  (six waveform bytes and 36 padding bytes); the prototype sent 41.
- A failed refresh invalidates panel state, forcing a clean reset on retry.
  SPI and BUSY read errors are propagated. Refreshes require a BUSY
  acknowledgement before being counted as successful.
- Partial refreshes preserve the 48,000-byte framebuffer and synchronize
  reference RAM after acknowledgement. The partial window covers the full
  800×480 frame, including its margins.
- Cards advance only after a successful refresh. Retries keep the same card
  and back off from 5 to 60 seconds. `current` identifies the last successfully
  displayed card; status adds `display_ready`, `displayed_position`,
  `refresh_failures`, `deck_revision`, and `uploading`.
- Reading an indexed card is synchronized with deck replacement. A completed
  render from an older deck cannot advance the new deck's position.
- Startup rebuilds the small index from the authoritative JSON deck, avoiding
  stale offsets after interrupted uploads. Temporary upload files are cleaned
  on failure and startup. Cached index reads and card reads have bounded sizes.
- Deck validation checks the whole JSON document, including separators and
  trailing data, and accepts `date` after `cards`. The top-level fields are
  exactly `date` and `cards`. HTTP JSON responses declare their content type.
- The framebuffer is reserved before networking allocations. The compressed
  font preserves every glyph and the rendered pixels.

A complete backup of the working Python installation is kept locally at
`firmware/backups/python-original.bin`; backups and credentials are ignored by
Git. To restore it:

```sh
../eink_python/.venv/bin/python -m esptool --chip esp32 \
  --port /dev/cu.wchusbserial1140 --baud 115200 \
  write-flash 0 firmware/backups/python-original.bin
```

To flash the built ELF (the bootloader must come from the same IDF build):

```sh
~/.cargo/bin/espflash flash --port /dev/cu.wchusbserial1140 --baud 230400 \
  --chip esp32 --flash-size 4mb --flash-mode dio --flash-freq 40mhz \
  --partition-table firmware/partitions.csv \
  --bootloader firmware/target/xtensa-esp32-espidf/release/bootloader.bin \
  firmware/target/xtensa-esp32-espidf/release/eink-japanese-rust
```

To build, flash, and open the serial monitor in one command, use:

```sh
bash firmware/build.sh run
```

This loads `firmware/.env`, activates ESP-IDF, and runs
`cargo +esp run --release` from `firmware/`. The configured runner flashes the
same partition table and bootloader.

The first install replaces the Python filesystem with SPIFFS. Preserve a full
flash backup before switching implementations. Later Rust flashes preserve
SPIFFS. Build artifacts contain the compiled Wi-Fi credentials and are local.

The board verification helper uploads a deck, rejects a malformed upload,
checks acknowledged refreshes and card IDs for 200 seconds, and observes the
three-minute clean refresh:

```sh
python3 tools/verify_board.py http://DEVICE_IP \
  decks/japanese_100.json --seconds 200
```

It checks API state and BUSY acknowledgements; it does not visually inspect
the panel.

## Repaired build on the board (2026-10-11)

Flashed with the IDF bootloader and the custom partition table. GPIO13 now
acknowledges the startup clean refresh and the first Japanese partial refresh;
`display_ready` is true and `error` is empty. The preserved 100-card deck is
installed, with the first card reported as `japanese:wa`. A malformed upload
returns HTTP 400 and leaves the installed deck and revision intact.

| Measurement | Original Rust prototype | Repaired Rust build |
| --- | ---: | ---: |
| App image | 1,992,480 B | 1,707,888 B |
| Embedded font | 902,460 B | 607,628 B |
| Filesystem used with the same deck | 18,323 B | 18,323 B |
| App + filesystem used | 2,010,803 B | 1,726,211 B |
| Largest free heap block | 110,592 B | 110,592 B |
| Free heap after restart with deck | 151,956 B | 152,212 B |
| Minimum free heap after restart | Not recorded | 150,364 B |

The repaired app is 14.3% smaller than the prototype. Compared with the Python
installation, its app is 77,696 bytes (4.8%) larger, while the current
Japanese-only app plus filesystem uses 1,263,853 fewer bytes. The feature-set
and allocator caveats in the original comparison still apply.

The 200-second API check passed: four successive cards (`wa`, `ga`, `wo`,
`ni`) were acknowledged, partial refreshes took 853–891 ms, and the periodic
clean refresh took 5,047 ms. Card reads took 1–3 ms and rendering took 11–29 ms.
Free heap during polling ranged from 149,944 to 152,180 bytes; the largest
block remained 110,592 bytes. The malformed upload check contributed to the
144,612-byte minimum heap watermark during that run.

After a physical restart the same 100-card deck loaded from SPIFFS and
`japanese:wa` completed a clean refresh in 5,045 ms with `display_ready=true`,
no error, and zero refresh failures. That status sample had 152,212 free heap
bytes and a 150,364-byte minimum watermark. The repaired Rust build is left
installed and serving `http://192.168.178.88`.

The software observed the panel's active-low BUSY acknowledgement and return
to idle; a visual inspection of the physical panel has not been performed.
Local evidence is saved in `firmware/backups/verify-board.log`,
`board-samples.json`, `reboot-status.json`, and the boot/flash/build logs.
The full Python backup has an accompanying `python-original.sha256` checksum.

## Glanceable learning layout

The included `decks/japanese_100.json` adds a contextual reading and direct
English gloss for every sentence word. The renderer places the reading above
the Japanese word and its English gloss directly below it. Particles retain
their literal grammatical role even when the resulting sequence is not natural
English. A separate natural translation follows the sentence. Individual kanji
and their short meaning cues use the remaining screen area.

The renderer chooses the largest layout that fits. Short examples use 64-pixel
Japanese and 32-pixel English. Longer examples wrap into two aligned rows and
use a compact side-by-side heading so the main learning text remains large.
Preview any card on the host with:

```sh
cargo run --offline --example render_card -- 1 card.pbm
```

Run `python3 tools/build_learning_deck.py` to regenerate the enriched deck from
the sibling source deck. See `LEARNING_DESIGN.md` for the evidence and the
limits of applying it to a passive display.

## High-resolution fonts

The current build replaces the scaled 32×32 Japanese glyphs with 64×64
masters. Large Japanese text now renders at native panel resolution instead
of doubling every source pixel. Small Japanese text uses a 2×2 majority
filter. Printable Latin text uses separate 8×16 and 16×32 raster fonts instead
of scaling a 5×7 uppercase-only table, so lowercase letters and punctuation
retain their shapes. Run `tools/build_fonts.py` with Pillow installed to
regenerate both source atlases.

This font update changes the partition boundary. Flash the included partition
table with the firmware and upload the deck again; data in the old SPIFFS
location is not retained.

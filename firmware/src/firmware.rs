use anyhow::{Result, anyhow, bail};
use embedded_hal::delay::DelayNs;
use embedded_svc::http::{Headers, Method};
use embedded_svc::io::Write;
use esp_idf_hal::delay::{Ets, FreeRtos};
use esp_idf_hal::gpio::{Gpio19, PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::spi::{Dma, SPI2, SpiDeviceDriver, SpiDriver, config};
use esp_idf_hal::units::*;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::{Configuration as HttpConfiguration, EspHttpServer};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{
    AuthMethod, BlockingWifi, ClientConfiguration, Configuration as WifiConfiguration, EspWifi,
};
use japanese_core::panel::Panel;
use japanese_core::{Card, DeckIndex, FRAME_BYTES, index_deck, read_card, render_card, save_index};
use serde_json::json;
use std::fs::{self, File};
use std::io::Write as StdWrite;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use std::time::{Duration, Instant};

const WIFI_SSID: &str = match option_env!("WIFI_SSID") {
    Some(v) => v,
    None => "",
};
const WIFI_PASSWORD: &str = match option_env!("WIFI_PASSWORD") {
    Some(v) => v,
    None => "",
};
const DECK: &str = "/spiffs/japanese.json";
const INDEX: &str = "/spiffs/japanese.idx";
const META: &str = "/spiffs/japanese.meta";
const UPLOAD: &str = "/spiffs/japanese.upload";
const INDEX_TMP: &str = "/spiffs/japanese.idx.tmp";
const META_TMP: &str = "/spiffs/japanese.meta.tmp";
const FONT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/japanese_font.jpf"));

#[derive(Default)]
struct Profile {
    upload_receive_ms: u32,
    upload_index_ms: u32,
    upload_total_ms: u32,
    card_read_ms: u32,
    render_ms: u32,
    refresh_ms: u32,
    clean_refresh_ms: u32,
    partial_refresh_ms: u32,
}

#[derive(Default)]
struct Runtime {
    deck: Option<DeckIndex>,
    position: usize,
    current: String,
    error: String,
    profile: Profile,
    display_ready: bool,
    refresh_failures: u32,
    displayed_position: usize,
}

struct Shared {
    runtime: Mutex<Runtime>,
    uploading: AtomicBool,
    fs_total: AtomicU32,
    fs_used: AtomicU32,
    revision: AtomicU32,
}

struct UploadGuard<'a>(&'a Shared);
impl Drop for UploadGuard<'_> {
    fn drop(&mut self) {
        for path in [UPLOAD, INDEX_TMP, META_TMP] {
            let _ = fs::remove_file(path);
        }
        if let Ok((total, used)) = spiffs_info() {
            self.0.fs_total.store(total, Ordering::Release);
            self.0.fs_used.store(used, Ordering::Release);
        }
        self.0.uploading.store(false, Ordering::Release);
    }
}

pub fn run() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    if WIFI_SSID.is_empty() || WIFI_PASSWORD.is_empty() {
        bail!("Set WIFI_SSID and WIFI_PASSWORD before building (see README.md)");
    }
    mount_storage()?;
    for path in [UPLOAD, INDEX_TMP, META_TMP] {
        let _ = fs::remove_file(path);
    }
    let runtime = load_runtime();
    let (fs_total, fs_used) = spiffs_info()?;
    // Reserve the largest allocation before networking fragments the heap.
    let mut frame = vec![0xff; FRAME_BYTES];
    let shared = Arc::new(Shared {
        runtime: Mutex::new(runtime),
        uploading: AtomicBool::new(false),
        fs_total: AtomicU32::new(fs_total),
        fs_used: AtomicU32::new(fs_used),
        revision: AtomicU32::new(0),
    });

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;
    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration {
        ssid: WIFI_SSID
            .try_into()
            .map_err(|_| anyhow!("WIFI_SSID must be at most 32 bytes"))?,
        password: WIFI_PASSWORD
            .try_into()
            .map_err(|_| anyhow!("WIFI_PASSWORD must be at most 64 bytes"))?,
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.connect()?;
    wifi.wait_netif_up()?;
    println!(
        "Japanese Rust API ready at http://{}",
        wifi.wifi().sta_netif().get_ip_info()?.ip
    );

    let spi_driver = SpiDriver::new::<SPI2>(
        peripherals.spi2,
        peripherals.pins.gpio18,
        peripherals.pins.gpio23,
        None::<Gpio19>,
        &config::DriverConfig::new().dma(Dma::Auto(4096)),
    )?;
    let spi = SpiDeviceDriver::new(
        &spi_driver,
        Some(peripherals.pins.gpio14),
        &config::Config::new().baudrate(4_u32.MHz().into()),
    )?;
    let dc = PinDriver::output(peripherals.pins.gpio25)?;
    let reset = PinDriver::output(peripherals.pins.gpio26)?;
    let busy = PinDriver::input(peripherals.pins.gpio13, Pull::Up)?;
    let mut panel = Panel::new(spi, dc, reset, busy, PanelDelay)?;
    let config = HttpConfiguration {
        max_sessions: 3,
        stack_size: 8192,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&config)?;
    register_routes(&mut server, shared.clone())?;

    let mut last_clean: Option<Instant> = None;
    let mut next_card = Instant::now();
    let mut seen_revision = shared.revision.load(Ordering::Acquire);
    loop {
        let revision = shared.revision.load(Ordering::Acquire);
        if revision != seen_revision {
            seen_revision = revision;
            next_card = Instant::now();
        }
        if Instant::now() >= next_card && !shared.uploading.load(Ordering::Acquire) {
            let clean_due = last_clean.is_none_or(|t| t.elapsed() >= Duration::from_secs(180));
            match next_card_frame(&shared, &mut frame) {
                Ok(Some(prepared)) => {
                    let start = Instant::now();
                    let clean = clean_due || !panel.ready;
                    match panel.show(&frame, clean) {
                        Ok(()) => {
                            let refresh_ms = start.elapsed().as_millis() as u32;
                            if let Ok(mut state) = shared.runtime.lock() {
                                state.profile.card_read_ms = prepared.read_ms;
                                state.profile.render_ms = prepared.render_ms;
                                state.display_ready = panel.ready;
                                state.refresh_failures = 0;
                                state.error.clear();
                                if shared.revision.load(Ordering::Acquire) == prepared.revision {
                                    state.current = format!("japanese:{}", prepared.id);
                                    state.displayed_position = prepared.number + 1;
                                    state.position = (prepared.number + 1) % prepared.total;
                                }
                                state.profile.refresh_ms = refresh_ms;
                                if clean {
                                    state.profile.clean_refresh_ms = refresh_ms;
                                    last_clean = Some(Instant::now());
                                } else {
                                    state.profile.partial_refresh_ms = refresh_ms;
                                }
                            }
                            next_card = Instant::now() + Duration::from_secs(60);
                        }
                        Err(error) => {
                            if let Ok(mut state) = shared.runtime.lock() {
                                state.error = error.to_string();
                                state.display_ready = panel.ready;
                                state.refresh_failures = state.refresh_failures.saturating_add(1);
                            }
                            next_card = Instant::now() + retry_delay(&shared);
                        }
                    }
                }
                Ok(None) => {
                    let blank = empty_card();
                    render_card(&mut frame, FONT, &blank, "", 0, 0)?;
                    match panel.show(&frame, true) {
                        Ok(()) => {
                            last_clean = Some(Instant::now());
                            if let Ok(mut state) = shared.runtime.lock() {
                                state.current = "waiting".into();
                                state.error.clear();
                                state.display_ready = panel.ready;
                                state.refresh_failures = 0;
                            }
                            next_card = Instant::now() + Duration::from_secs(3600);
                        }
                        Err(error) => {
                            if let Ok(mut state) = shared.runtime.lock() {
                                state.error = error.to_string();
                                state.display_ready = panel.ready;
                                state.refresh_failures = state.refresh_failures.saturating_add(1);
                            }
                            next_card = Instant::now() + retry_delay(&shared);
                        }
                    }
                }
                Err(error) => {
                    if let Ok(mut state) = shared.runtime.lock() {
                        state.error = error.to_string();
                        state.display_ready = panel.ready;
                        state.refresh_failures = state.refresh_failures.saturating_add(1);
                    }
                    next_card = Instant::now() + retry_delay(&shared);
                }
            }
        }
        FreeRtos::delay_ms(100);
        let _ = &server;
        let _ = &wifi;
    }
}

fn mount_storage() -> Result<(u32, u32)> {
    let base = b"/spiffs\0";
    let label = b"storage\0";
    let conf = esp_idf_sys::esp_vfs_spiffs_conf_t {
        base_path: base.as_ptr(),
        partition_label: label.as_ptr(),
        max_files: 5,
        format_if_mount_failed: true,
    };
    let err = unsafe { esp_idf_sys::esp_vfs_spiffs_register(&conf) };
    if err != esp_idf_sys::ESP_OK {
        bail!("SPIFFS mount failed: {err}");
    }
    let mut total = 0usize;
    let mut used = 0usize;
    let err = unsafe { esp_idf_sys::esp_spiffs_info(label.as_ptr(), &mut total, &mut used) };
    if err != esp_idf_sys::ESP_OK {
        bail!("SPIFFS info failed: {err}");
    }
    println!(
        "SPIFFS: total={} used={} free={}",
        total,
        used,
        total.saturating_sub(used)
    );
    Ok((total as u32, used as u32))
}

fn spiffs_info() -> Result<(u32, u32)> {
    let label = b"storage\0";
    let mut total = 0usize;
    let mut used = 0usize;
    let err = unsafe { esp_idf_sys::esp_spiffs_info(label.as_ptr(), &mut total, &mut used) };
    if err != esp_idf_sys::ESP_OK {
        bail!("SPIFFS info failed: {err}");
    }
    Ok((total as u32, used as u32))
}

fn load_runtime() -> Runtime {
    let mut state = Runtime::default();
    if std::path::Path::new(DECK).exists() {
        // The JSON file is authoritative. Rebuild small caches after any interrupted
        // upload rather than trusting offsets that may belong to a previous deck.
        match index_deck(DECK) {
            Ok(index) => {
                let _ = save_index(INDEX, &index);
                let _ = fs::write(META, format!("{}\n{}\n", index.date, index.cards.len()));
                state.deck = Some(index);
            }
            Err(error) => state.error = format!("stored deck: {error}"),
        }
    }
    state
}

fn retry_delay(shared: &Shared) -> Duration {
    let failures = shared
        .runtime
        .lock()
        .map(|s| s.refresh_failures)
        .unwrap_or(1);
    // Retry the same card, but avoid continuously resetting a disconnected panel.
    Duration::from_secs((5u64 << failures.saturating_sub(1).min(4)).min(60))
}

fn empty_card() -> Card {
    Card {
        id: "waiting".into(),
        kind: "info".into(),
        japanese: "日本語".into(),
        reading: "にほんご".into(),
        meaning: "SEND A DECK TO BEGIN".into(),
        example: japanese_core::Example {
            ja: "POST /api/japanese/daily".into(),
            en: "One new card every minute".into(),
            words: Vec::new(),
        },
        kanji: Vec::new(),
    }
}

struct PreparedCard {
    id: String,
    number: usize,
    total: usize,
    revision: u32,
    read_ms: u32,
    render_ms: u32,
}

fn next_card_frame(shared: &Shared, frame: &mut [u8]) -> Result<Option<PreparedCard>> {
    let start = Instant::now();
    let (card, date, number, total, revision) = {
        let state = shared
            .runtime
            .lock()
            .map_err(|_| anyhow!("runtime lock poisoned"))?;
        let Some(deck) = state.deck.as_ref() else {
            return Ok(None);
        };
        let number = state.position % deck.cards.len();
        // Hold the lock through this small read so an upload cannot replace the
        // file between choosing an offset and opening the matching JSON file.
        let card = read_card(DECK, deck.cards[number])?;
        (
            card,
            deck.date.clone(),
            number,
            deck.cards.len(),
            shared.revision.load(Ordering::Acquire),
        )
    };
    let read_ms = start.elapsed().as_millis() as u32;
    let start = Instant::now();
    render_card(frame, FONT, &card, &date, number + 1, total)?;
    Ok(Some(PreparedCard {
        id: card.id,
        number,
        total,
        revision,
        read_ms,
        render_ms: start.elapsed().as_millis() as u32,
    }))
}

fn register_routes(server: &mut EspHttpServer<'static>, shared: Arc<Shared>) -> Result<()> {
    let status_shared = shared.clone();
    server.fn_handler("/api/status", Method::Get, move |request| {
        let body = status_json(&status_shared);
        request
            .into_response(200, None, &[("Content-Type", "application/json")])?
            .write_all(body.as_bytes())
    })?;

    server.fn_handler(
        "/api/japanese/daily",
        Method::Post,
        move |mut request| -> Result<()> {
            let length = request.content_len().unwrap_or(0) as usize;
            if length == 0 || length > japanese_core::MAX_DECK_BYTES as usize {
                let body = r#"{"error":"Japanese deck must be 1 to 131072 bytes"}"#;
                request
                    .into_response(413, None, &[("Content-Type", "application/json")])?
                    .write_all(body.as_bytes())?;
                return Ok(());
            }
            let free = shared
                .fs_total
                .load(Ordering::Acquire)
                .saturating_sub(shared.fs_used.load(Ordering::Acquire));
            if free < length as u32 + 4096 {
                let body = r#"{"error":"Not enough flash storage for this deck"}"#;
                request
                    .into_response(507, None, &[("Content-Type", "application/json")])?
                    .write_all(body.as_bytes())?;
                return Ok(());
            }
            if shared
                .uploading
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                let body = r#"{"error":"Another upload is in progress"}"#;
                request
                    .into_response(409, None, &[("Content-Type", "application/json")])?
                    .write_all(body.as_bytes())?;
                return Ok(());
            }
            let _guard = UploadGuard(&shared);
            let total_start = Instant::now();
            let receive_start = Instant::now();
            let mut file = File::create(UPLOAD)?;
            let mut remaining = length;
            let mut chunk = [0u8; 512];
            while remaining > 0 {
                let want = remaining.min(chunk.len());
                let amount = request.read(&mut chunk[..want])?;
                if amount == 0 {
                    bail!("incomplete request body");
                }
                file.write_all(&chunk[..amount])?;
                remaining -= amount;
            }
            file.sync_all()?;
            drop(file);
            let receive_ms = receive_start.elapsed().as_millis() as u32;
            let index_start = Instant::now();
            let index = match index_deck(UPLOAD) {
                Ok(index) => index,
                Err(error) => {
                    let _ = fs::remove_file(UPLOAD);
                    let body = json!({"error":error.to_string()}).to_string();
                    request
                        .into_response(400, None, &[("Content-Type", "application/json")])?
                        .write_all(body.as_bytes())?;
                    return Ok(());
                }
            };
            save_index(INDEX_TMP, &index)?;
            fs::write(META_TMP, format!("{}\n{}\n", index.date, index.cards.len()))?;
            let index_ms = index_start.elapsed().as_millis() as u32;
            let count = index.cards.len();
            {
                let mut state = shared
                    .runtime
                    .lock()
                    .map_err(|_| anyhow!("runtime lock poisoned"))?;
                fs::rename(UPLOAD, DECK)?;
                // Caches are recoverable; a committed JSON deck remains usable
                // if a cache rename fails or power is lost here.
                let _ = fs::rename(INDEX_TMP, INDEX);
                let _ = fs::rename(META_TMP, META);
                state.deck = Some(index);
                state.position = 0;
                state.profile.upload_receive_ms = receive_ms;
                state.profile.upload_index_ms = index_ms;
                state.profile.upload_total_ms = total_start.elapsed().as_millis() as u32;
                shared.revision.fetch_add(1, Ordering::AcqRel);
            }
            let (fs_total, fs_used) = spiffs_info()?;
            shared.fs_total.store(fs_total, Ordering::Release);
            shared.fs_used.store(fs_used, Ordering::Release);
            let body = json!({"saved":true,"cards":count,"mode":"japanese"}).to_string();
            request
                .into_response(200, None, &[("Content-Type", "application/json")])?
                .write_all(body.as_bytes())?;
            Ok(())
        },
    )?;
    Ok(())
}

fn status_json(shared: &Shared) -> String {
    let state = shared.runtime.lock().ok();
    let heap_free =
        unsafe { esp_idf_sys::heap_caps_get_free_size(esp_idf_sys::MALLOC_CAP_8BIT as u32) };
    let heap_largest = unsafe {
        esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_8BIT as u32)
    };
    let heap_min = unsafe {
        esp_idf_sys::heap_caps_get_minimum_free_size(esp_idf_sys::MALLOC_CAP_8BIT as u32)
    };
    let state = state.as_deref();
    json!({
        "mode":"japanese",
        "current":state.map(|s| s.current.as_str()).unwrap_or(""),
        "display_ready":state.map(|s| s.display_ready).unwrap_or(false),
        "displayed_position":state.map(|s| s.displayed_position).unwrap_or(0),
        "refresh_failures":state.map(|s| s.refresh_failures).unwrap_or(0),
        "deck_revision":shared.revision.load(Ordering::Acquire),
        "uploading":shared.uploading.load(Ordering::Acquire),
        "error":state.map(|s| s.error.as_str()).unwrap_or(""),
        "cards":state.and_then(|s| s.deck.as_ref()).map(|d| d.cards.len()).unwrap_or(0),
        "heap_free_bytes":heap_free,
        "heap_largest_block_bytes":heap_largest,
        "heap_min_free_bytes":heap_min,
        "free_bytes":shared.fs_total.load(Ordering::Acquire).saturating_sub(shared.fs_used.load(Ordering::Acquire)),
        "profile":state.map(|s| json!({
            "upload_receive_ms":s.profile.upload_receive_ms,
            "upload_index_ms":s.profile.upload_index_ms,
            "upload_total_ms":s.profile.upload_total_ms,
            "card_read_ms":s.profile.card_read_ms,
            "render_ms":s.profile.render_ms,
            "refresh_ms":s.profile.refresh_ms,
            "clean_refresh_ms":s.profile.clean_refresh_ms,
            "partial_refresh_ms":s.profile.partial_refresh_ms
        })).unwrap_or_else(|| json!({}))
    }).to_string()
}

/// Preserve short panel timing while yielding for longer waits.
struct PanelDelay;
impl DelayNs for PanelDelay {
    fn delay_ns(&mut self, ns: u32) {
        let us = ns.saturating_add(999) / 1000;
        if us <= 2000 {
            Ets::delay_us(us);
        } else {
            FreeRtos::delay_ms(us.saturating_add(999) / 1000);
        }
    }
}

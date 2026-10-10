// SPDX-License-Identifier: GPL-3.0-only
//! Waveshare 7.5-inch V2 driver, using the working MicroPython command sequence.
//! Based on Waveshare epd7in5_V2 and GxEPD2_750_T7; see ../licenses/.
//! The supplied delay must preserve the 2 ms reset pulse (not round it to an OS tick).
use crate::FRAME_BYTES;
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::spi::SpiDevice;
use std::io::{self, Result};
use std::time::{Duration, Instant};

pub struct Panel<S, DC, RST, BUSY, D> {
    spi: S,
    dc: DC,
    reset: RST,
    busy: BUSY,
    pub ready: bool,
    delay: D,
    busy_timeout: Duration,
}
impl<S, DC, RST, BUSY, D> Panel<S, DC, RST, BUSY, D>
where
    S: SpiDevice<u8>,
    DC: OutputPin,
    RST: OutputPin,
    BUSY: InputPin,
    D: DelayNs,
{
    pub fn new(spi: S, dc: DC, mut reset: RST, busy: BUSY, delay: D) -> Result<Self> {
        reset
            .set_high()
            .map_err(|_| io::Error::other("reset pin failed"))?;
        Ok(Self {
            spi,
            dc,
            reset,
            busy,
            ready: false,
            delay,
            busy_timeout: Duration::from_secs(30),
        })
    }
    fn command(&mut self, value: u8, data: Option<&[u8]>) -> Result<()> {
        self.dc
            .set_low()
            .map_err(|_| io::Error::other("DC pin failed"))?;
        self.spi
            .write(&[value])
            .map_err(|_| io::Error::other("SPI command failed"))?;
        if let Some(data) = data {
            self.dc
                .set_high()
                .map_err(|_| io::Error::other("DC pin failed"))?;
            self.spi
                .write(data)
                .map_err(|_| io::Error::other("SPI data failed"))?;
        }
        Ok(())
    }
    fn wait_idle(&mut self, fast: bool, require_busy: bool, stage: &str) -> Result<()> {
        let start = Instant::now();
        let mut observed = false;
        // Poll at one FreeRTOS tick or slower so a stuck BUSY line cannot
        // starve the idle task and trigger the watchdog before our timeout.
        let delay = if fast { 10 } else { 20 };
        if !fast {
            self.command(0x71, None)?;
        }
        while !self
            .busy
            .is_high()
            .map_err(|_| io::Error::other("BUSY pin read failed"))?
        {
            observed = true;
            if start.elapsed() > self.busy_timeout {
                return Err(io::Error::other(format!(
                    "display BUSY timeout during {stage}; GPIO13 stayed low"
                )));
            }
            self.delay.delay_ms(delay);
            if !fast {
                self.command(0x71, None)?;
            }
        }
        self.delay.delay_ms(delay);
        if require_busy && !observed {
            return Err(io::Error::other(format!(
                "display did not acknowledge {stage}"
            )));
        }
        Ok(())
    }
    fn configure(&mut self) -> Result<()> {
        self.command(0x01, Some(&[0x07, 0x07, 0x3f, 0x3f]))?;
        self.command(0x00, Some(&[0x1f]))?;
        self.command(0x61, Some(&[0x03, 0x20, 0x01, 0xe0]))?;
        self.command(0x15, Some(&[0x00]))?;
        self.command(0x50, Some(&[0x29, 0x07]))?;
        self.command(0x60, Some(&[0x22]))?;
        Ok(())
    }
    fn send_data(&mut self, bytes: &[u8]) -> Result<()> {
        self.dc
            .set_high()
            .map_err(|_| io::Error::other("DC pin failed"))?;
        self.spi
            .write(bytes)
            .map_err(|_| io::Error::other("SPI frame failed"))?;
        Ok(())
    }
    fn clean(&mut self, frame: &[u8]) -> Result<()> {
        self.ready = false;
        println!("DISPLAY: clean refresh");
        self.reset
            .set_high()
            .map_err(|_| io::Error::other("reset pin failed"))?;
        self.delay.delay_ms(20);
        self.reset
            .set_low()
            .map_err(|_| io::Error::other("reset pin failed"))?;
        self.delay.delay_ms(2);
        self.reset
            .set_high()
            .map_err(|_| io::Error::other("reset pin failed"))?;
        self.delay.delay_ms(20);
        self.configure()?;
        self.command(0x04, None)?;
        self.delay.delay_ms(100);
        self.wait_idle(false, false, "power-on")?;
        self.command(0x10, None)?;
        let white = [0xff; 1000];
        for _ in 0..(FRAME_BYTES / white.len()) {
            self.send_data(&white)?;
        }
        self.command(0x13, None)?;
        self.send_data(frame)?;
        self.command(0x12, None)?;
        self.delay.delay_ms(100);
        self.wait_idle(false, true, "clean-refresh")?;
        self.configure()?;
        self.command(0x00, Some(&[0x3f]))?;
        self.command(0x82, Some(&[0x26]))?;
        self.command(0x50, Some(&[0x39, 0x07]))?;
        for (register, transition) in [0, 0, 0x5a, 0x84, 0, 0].iter().enumerate() {
            let mut data = [0u8; 42];
            data[0] = *transition;
            data[1..5].copy_from_slice(&[10, 2, 10, 2]);
            data[5] = 1;
            self.command(0x20 + register as u8, Some(&data))?;
        }
        self.ready = true;
        Ok(())
    }
    fn write_region(&mut self, frame: &[u8]) -> Result<()> {
        self.command(0x91, None)?;
        self.command(
            0x90,
            Some(&[0x00, 0x00, 0x03, 0x1f, 0x00, 0x00, 0x01, 0xdf, 0x01]),
        )?;
        self.command(0x13, None)?;
        self.send_data(frame)?;
        self.command(0x92, None)
    }
    pub fn show(&mut self, frame: &[u8], clean: bool) -> Result<()> {
        if frame.len() != FRAME_BYTES {
            return Err(io::Error::other("framebuffer must be exactly 48000 bytes"));
        }
        let result = if clean || !self.ready {
            self.clean(frame)
        } else {
            self.partial(frame)
        };
        if result.is_err() {
            // A failed transfer leaves both controller RAM and waveforms uncertain.
            self.ready = false;
        }
        result
    }

    fn partial(&mut self, frame: &[u8]) -> Result<()> {
        self.write_region(frame)?;
        self.command(0x12, None)?;
        self.delay.delay_ms(2);
        self.wait_idle(true, true, "partial-refresh")?;
        // Sync reference RAM only after the panel acknowledges the refresh.
        self.write_region(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal::digital::{ErrorKind as PinError, ErrorType as PinErrorType};
    use embedded_hal::spi::{ErrorKind as SpiError, ErrorType as SpiErrorType, Operation};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::result::Result;

    #[derive(Debug, PartialEq)]
    enum Event {
        Reset(bool),
        Delay(u32),
        Write(bool, Vec<u8>),
    }
    #[derive(Default)]
    struct Trace {
        events: Vec<Event>,
        dc: bool,
        busy: VecDeque<Result<bool, PinError>>,
        fail_spi: bool,
    }
    type Shared = Rc<RefCell<Trace>>;
    struct Spi(Shared);
    impl SpiErrorType for Spi {
        type Error = SpiError;
    }
    impl SpiDevice for Spi {
        fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), SpiError> {
            let mut trace = self.0.borrow_mut();
            if trace.fail_spi {
                trace.fail_spi = false;
                return Err(SpiError::Other);
            }
            for operation in operations {
                match operation {
                    Operation::Write(bytes) => {
                        let dc = trace.dc;
                        trace.events.push(Event::Write(dc, bytes.to_vec()));
                    }
                    _ => panic!("driver only writes"),
                }
            }
            Ok(())
        }
    }
    struct Pin {
        trace: Shared,
        reset: bool,
    }
    impl PinErrorType for Pin {
        type Error = PinError;
    }
    impl OutputPin for Pin {
        fn set_low(&mut self) -> Result<(), PinError> {
            self.set(false)
        }
        fn set_high(&mut self) -> Result<(), PinError> {
            self.set(true)
        }
    }
    impl Pin {
        fn set(&self, high: bool) -> Result<(), PinError> {
            let mut trace = self.trace.borrow_mut();
            if self.reset {
                trace.events.push(Event::Reset(high));
            } else {
                trace.dc = high;
            }
            Ok(())
        }
    }
    struct Busy(Shared);
    impl PinErrorType for Busy {
        type Error = PinError;
    }
    impl InputPin for Busy {
        fn is_high(&mut self) -> Result<bool, PinError> {
            self.0.borrow_mut().busy.pop_front().unwrap_or(Ok(true))
        }
        fn is_low(&mut self) -> Result<bool, PinError> {
            self.is_high().map(|v| !v)
        }
    }
    struct Delay(Shared);
    impl DelayNs for Delay {
        fn delay_ns(&mut self, ns: u32) {
            self.0.borrow_mut().events.push(Event::Delay(ns));
        }
    }
    type Driver = Panel<Spi, Pin, Pin, Busy, Delay>;
    fn driver() -> (Driver, Shared) {
        let trace = Shared::default();
        let panel = Panel::new(
            Spi(trace.clone()),
            Pin {
                trace: trace.clone(),
                reset: false,
            },
            Pin {
                trace: trace.clone(),
                reset: true,
            },
            Busy(trace.clone()),
            Delay(trace.clone()),
        )
        .unwrap();
        (panel, trace)
    }
    fn clean_ready(panel: &mut Driver, trace: &Shared, frame: &[u8]) {
        trace
            .borrow_mut()
            .busy
            .extend([Ok(true), Ok(false), Ok(true)]);
        panel.show(frame, true).unwrap();
        assert!(panel.ready);
    }
    #[test]
    fn reset_uses_two_milliseconds_and_complete_waveforms() {
        let (mut panel, trace) = driver();
        let frame = vec![0xff; FRAME_BYTES];
        clean_ready(&mut panel, &trace, &frame);
        let trace = trace.borrow();
        assert_eq!(
            &trace.events[..7],
            &[
                Event::Reset(true),
                Event::Reset(true),
                Event::Delay(20_000_000),
                Event::Reset(false),
                Event::Delay(2_000_000),
                Event::Reset(true),
                Event::Delay(20_000_000),
            ]
        );
        for register in 0x20..=0x25 {
            let at = trace
                .events
                .iter()
                .position(|e| *e == Event::Write(false, vec![register]))
                .unwrap();
            match &trace.events[at + 1] {
                Event::Write(true, data) => {
                    assert_eq!(data.len(), 42);
                    assert_eq!(&data[1..6], &[10, 2, 10, 2, 1]);
                }
                _ => panic!("missing waveform data"),
            }
        }
    }
    #[test]
    fn partial_refresh_preserves_frame_and_syncs_reference_ram() {
        let (mut panel, trace) = driver();
        let frame: Vec<_> = (0..FRAME_BYTES).map(|i| i as u8).collect();
        clean_ready(&mut panel, &trace, &frame);
        trace.borrow_mut().events.clear();
        trace.borrow_mut().busy.extend([Ok(false), Ok(true)]);
        panel.show(&frame, false).unwrap();
        let trace = trace.borrow();
        let transfers = trace
            .events
            .iter()
            .filter(|e| **e == Event::Write(true, frame.clone()))
            .count();
        assert_eq!(transfers, 2);
        assert!(
            trace
                .events
                .contains(&Event::Write(true, vec![0, 0, 3, 0x1f, 0, 0, 1, 0xdf, 1]))
        );
    }
    #[test]
    fn failed_partial_forces_clean_reset_before_retry() {
        let (mut panel, trace) = driver();
        let frame = vec![0xff; FRAME_BYTES];
        clean_ready(&mut panel, &trace, &frame);
        trace.borrow_mut().fail_spi = true;
        assert!(panel.show(&frame, false).is_err());
        assert!(!panel.ready);
        trace.borrow_mut().events.clear();
        trace
            .borrow_mut()
            .busy
            .extend([Ok(true), Ok(false), Ok(true)]);
        panel.show(&frame, false).unwrap();
        assert_eq!(trace.borrow().events[0], Event::Reset(true));
    }
    #[test]
    fn busy_read_errors_and_missing_acknowledgements_are_failures() {
        let (mut panel, trace) = driver();
        let frame = vec![0xff; FRAME_BYTES];
        trace.borrow_mut().busy.push_back(Err(PinError::Other));
        assert!(
            panel
                .show(&frame, true)
                .unwrap_err()
                .to_string()
                .contains("BUSY pin read")
        );
        assert!(!panel.ready);
        assert!(
            panel
                .show(&frame, true)
                .unwrap_err()
                .to_string()
                .contains("acknowledge")
        );
        assert!(!panel.ready);
        panel.busy_timeout = Duration::ZERO;
        trace.borrow_mut().busy.push_back(Ok(false));
        assert!(
            panel
                .show(&frame, true)
                .unwrap_err()
                .to_string()
                .contains("power-on")
        );
        assert!(!panel.ready);
        trace.borrow_mut().events.clear();
        assert!(panel.show(&frame[..1], true).is_err());
        assert!(trace.borrow().events.is_empty());
    }
}

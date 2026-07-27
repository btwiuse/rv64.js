//! Goldfish real-time clock, as exposed by QEMU's RISC-V `virt` machine.
//!
//! The CLINT is a monotonic timer and deliberately starts at zero. Linux needs
//! a separate RTC to seed `CLOCK_REALTIME`; otherwise a perfectly modern guest
//! still believes it is 1970 and rejects every public TLS certificate.

const TIME_LOW: u64 = 0x00;
const TIME_HIGH: u64 = 0x04;
const ALARM_LOW: u64 = 0x08;
const ALARM_HIGH: u64 = 0x0c;
const IRQ_ENABLED: u64 = 0x10;
const CLEAR_ALARM: u64 = 0x14;
const ALARM_STATUS: u64 = 0x18;
const CLEAR_INTERRUPT: u64 = 0x1c;

/// The eight 32-bit registers implemented by Linux's `rtc-goldfish` driver.
pub struct GoldfishRtc {
    host_ns: u64,
    guest_offset_ns: i128,
    latched_time_high: u32,
    write_time_high: u32,
    alarm_high: u32,
    alarm_ns: u64,
    alarm_armed: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

impl GoldfishRtc {
    pub fn new() -> Self {
        Self {
            host_ns: 0,
            guest_offset_ns: 0,
            latched_time_high: 0,
            write_time_high: 0,
            alarm_high: 0,
            alarm_ns: 0,
            alarm_armed: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    /// Refresh the host's Unix-epoch clock. A guest `RTC_SET_TIME` adjustment
    /// is retained as an offset across later host refreshes.
    pub fn set_host_time_ns(&mut self, ns: u64) {
        self.host_ns = ns;
        self.update_alarm();
    }

    pub fn time_ns(&self) -> u64 {
        (self.host_ns as i128 + self.guest_offset_ns).clamp(0, u64::MAX as i128) as u64
    }

    pub fn read(&mut self, off: u64) -> u32 {
        match off {
            // Reading LOW latches HIGH, which makes a rollover-safe pair.
            TIME_LOW => {
                let now = self.time_ns();
                self.latched_time_high = (now >> 32) as u32;
                now as u32
            }
            TIME_HIGH => self.latched_time_high,
            ALARM_LOW => self.alarm_ns as u32,
            ALARM_HIGH => (self.alarm_ns >> 32) as u32,
            IRQ_ENABLED => self.irq_enabled as u32,
            ALARM_STATUS => self.alarm_armed as u32,
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u64, val: u32) {
        match off {
            // Linux writes HIGH then LOW; LOW commits the complete timestamp.
            TIME_HIGH => self.write_time_high = val,
            TIME_LOW => {
                let requested = ((self.write_time_high as u64) << 32) | val as u64;
                self.guest_offset_ns = requested as i128 - self.host_ns as i128;
                self.update_alarm();
            }
            ALARM_HIGH => self.alarm_high = val,
            // LOW commits and arms the one-shot alarm.
            ALARM_LOW => {
                self.alarm_ns = ((self.alarm_high as u64) << 32) | val as u64;
                self.alarm_armed = true;
                self.update_alarm();
            }
            IRQ_ENABLED => {
                self.irq_enabled = val != 0;
                self.update_alarm();
            }
            CLEAR_ALARM => {
                self.alarm_armed = false;
                self.irq_pending = false;
            }
            CLEAR_INTERRUPT => self.irq_pending = false,
            _ => {}
        }
    }

    pub fn irq(&self) -> bool {
        self.irq_pending
    }

    fn update_alarm(&mut self) {
        if self.alarm_armed && self.irq_enabled && self.time_ns() >= self.alarm_ns {
            self.alarm_armed = false;
            self.irq_pending = true;
        }
    }
}

impl Default for GoldfishRtc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_low_latches_high_and_guest_set_is_an_offset() {
        let mut rtc = GoldfishRtc::new();
        rtc.set_host_time_ns(0x1234_5678_ffff_fff0);
        assert_eq!(rtc.read(TIME_LOW), 0xffff_fff0);
        rtc.set_host_time_ns(0x2234_5678_0000_0010);
        assert_eq!(rtc.read(TIME_HIGH), 0x1234_5678);

        rtc.write(TIME_HIGH, 3);
        rtc.write(TIME_LOW, 7);
        assert_eq!(rtc.time_ns(), (3u64 << 32) | 7);
        rtc.set_host_time_ns(0x2234_5678_0000_0110);
        assert_eq!(rtc.time_ns(), (3u64 << 32) | 0x107);
    }

    #[test]
    fn alarm_is_one_shot_and_interrupt_is_acknowledgeable() {
        let mut rtc = GoldfishRtc::new();
        rtc.set_host_time_ns(100);
        rtc.write(ALARM_HIGH, 0);
        rtc.write(ALARM_LOW, 200);
        rtc.write(IRQ_ENABLED, 1);
        assert_eq!(rtc.read(ALARM_STATUS), 1);
        assert!(!rtc.irq());

        rtc.set_host_time_ns(200);
        assert!(rtc.irq());
        assert_eq!(rtc.read(ALARM_STATUS), 0);
        rtc.write(CLEAR_INTERRUPT, 1);
        assert!(!rtc.irq());
    }
}

//! Giao thức USB của ZK9500, giải mã từ lưu lượng của plugin Windows (ZKFPSensors\*.dll)
//! bắt bằng USBPcap trong lúc ZKFingerSDK 5.3 chụp vân tay.
//!
//! Toàn bộ là vendor control transfer (bmRequestType 0x40 OUT / 0xC0 IN) trên EP0, cộng một
//! bulk IN 0x82 chở ảnh. Ảnh qua USB đã là ảnh cuối cùng: so từng byte với ảnh libzkfp.dll đưa
//! cho thuật toán thì trùng 100% (plugin Windows không hiệu chỉnh gì thêm với ZK9500).

use rusb::{Context, DeviceHandle, Direction, Recipient, RequestType, UsbContext};
use std::time::Duration;

pub const VID: u16 = 0x1b55;
/// Chỉ những PID đã kiểm chứng giao thức. 0x0120 (đầu đọc SilkID khác) dùng giao thức của
/// libsilkidcap.so gốc, không phải giao thức này.
pub const SUPPORTED_PIDS: &[u16] = &[0x0124];

const EP_IMAGE: u8 = 0x82;
const CTRL_TIMEOUT: Duration = Duration::from_millis(1000);
const IMAGE_TIMEOUT: Duration = Duration::from_millis(3000);

const REQ_START: u8 = 0xE0; // OUT, không dữ liệu: bước đầu tiên sau khi claim interface
const REQ_SYNC: u8 = 0x80; // OUT, 16 byte 0; thiết bị trả lại 4 byte 0 trên EP 0x82
const REQ_GPIO_SET: u8 = 0xE1; // OUT, wValue = giá trị, wIndex = chân/LED
const REQ_GPIO_GET: u8 = 0xE2; // IN
const REQ_SENSOR_REG: u8 = 0xE3; // OUT, wValue = giá trị, wIndex = thanh ghi CMOS
const REQ_EEPROM: u8 = 0xE7; // IN 1 byte, wIndex = địa chỉ (ZK9500 chỉ trả lời từng byte một)
const REQ_FINGER: u8 = 0xEA; // IN 1 byte: 1 = có ngón tay (ảnh sẵn sàng trên EP 0x82)
const REQ_LICENSE: u8 = 0xF3; // IN 4 byte: hỏi-đáp bản quyền của thuật toán zkfinger10

/// Thông số chụp đọc từ EEPROM (0x30..0x3b và 0x70..0x75). Giá trị mặc định là của chính
/// đầu đọc dùng để giải mã, chỉ dùng khi EEPROM đọc ra vô lý.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSettings {
    pub exposure: u16,
    pub gain_red: u16,
    pub gain_green1: u16,
    pub gain_green2: u16,
    pub gain_blue: u16,
    /// Main1, Main2, Side1, Side2, Anti1, Anti2 — gửi qua GPIO 0..5.
    pub leds: [u8; 6],
}

impl Default for CaptureSettings {
    fn default() -> Self {
        CaptureSettings {
            exposure: 1200,
            gain_red: 200,
            gain_green1: 200,
            gain_green2: 182,
            gain_blue: 200,
            leds: [200, 200, 175, 175, 120, 120],
        }
    }
}

impl CaptureSettings {
    /// `cam` = EEPROM 0x30..0x3b (6 x u16 LE: exposure, R, G1, G2, B, checksum),
    /// `leds` = EEPROM 0x70..0x75.
    pub fn from_eeprom(cam: &[u8; 12], leds: &[u8; 6]) -> (Self, bool) {
        let w = |i: usize| u16::from_le_bytes([cam[2 * i], cam[2 * i + 1]]);
        let (exposure, r, g1, g2, b, sum) = (w(0), w(1), w(2), w(3), w(4), w(5));
        let total = exposure as u32 + r as u32 + g1 as u32 + g2 as u32 + b as u32;
        // Đầu đọc mẫu: tổng 1982, ô checksum 1983. Chấp nhận cả hai kiểu (tổng, tổng+1) thay vì
        // đoán quy tắc chính xác; ngoài ra thì coi EEPROM hỏng và dùng mặc định.
        let ok = (sum as u32 == total || sum as u32 == total + 1)
            && (100..=10000).contains(&exposure)
            && leds.iter().all(|&l| l > 0);
        if !ok {
            return (Self::default(), false);
        }
        (
            CaptureSettings {
                exposure,
                gain_red: r,
                gain_green1: g1,
                gain_green2: g2,
                gain_blue: b,
                leds: *leds,
            },
            true,
        )
    }
}

/// Kích thước ảnh theo model. Hiện chỉ có ZK9500: 300 x 375 @ 500 dpi (khớp
/// ZKFPM_GetParameters 1/2/3 trên Windows và độ dài bulk 112500 byte).
pub fn image_geometry(_pid: u16) -> (u16, u16, u32) {
    (300, 375, 500)
}

pub struct Device {
    handle: DeviceHandle<Context>,
    pub vid: u16,
    pub pid: u16,
    pub rev: u16,
    pub width: u16,
    pub height: u16,
    pub dpi: u32,
    pub manufacturer: String,
    pub model: String,
    pub serial: String,
    /// EEPROM 0xe8..0xf7 — vùng "custom SN" mà SDK gốc trả cho tham số 1104 (thường rỗng).
    pub custom: [u8; 16],
    pub settings: CaptureSettings,
    pub settings_from_eeprom: bool,
    image: Vec<u8>,
    started: bool,
}

fn vendor(dir: Direction) -> u8 {
    rusb::request_type(dir, RequestType::Vendor, Recipient::Device)
}

impl Device {
    pub fn open(dev: &rusb::Device<Context>) -> rusb::Result<Device> {
        let desc = dev.device_descriptor()?;
        let handle = dev.open()?;
        // Không có kernel driver nào cho 1b55:0124, nhưng nếu có thì phải tách ra mới claim được.
        if let Ok(true) = handle.kernel_driver_active(0) {
            let _ = handle.detach_kernel_driver(0);
        }
        // SET_CONFIGURATION khi đã ở config 1 là một lần reset nhẹ -> chỉ gửi khi cần.
        if handle.active_configuration().unwrap_or(0) != 1 {
            handle.set_active_configuration(1)?;
        }
        handle.claim_interface(0)?;

        let ascii = |idx: Option<u8>| {
            idx.and_then(|i| handle.read_string_descriptor_ascii(i).ok())
                .unwrap_or_default()
        };
        let manufacturer = ascii(desc.manufacturer_string_index());
        let model = ascii(desc.product_string_index());
        let serial = ascii(desc.serial_number_string_index());

        let pid = desc.product_id();
        let (width, height, dpi) = image_geometry(pid);
        let v = desc.device_version();
        let rev = ((v.major() as u16) << 8) | ((v.minor() as u16) << 4) | v.sub_minor() as u16;

        let mut d = Device {
            handle,
            vid: desc.vendor_id(),
            pid,
            rev,
            width,
            height,
            dpi,
            manufacturer,
            model,
            serial,
            custom: [0; 16],
            settings: CaptureSettings::default(),
            settings_from_eeprom: false,
            image: vec![0; width as usize * height as usize],
            started: false,
        };
        d.start()?;
        Ok(d)
    }

    fn out(&self, req: u8, value: u16, index: u16, data: &[u8]) -> rusb::Result<()> {
        self.handle
            .write_control(vendor(Direction::Out), req, value, index, data, CTRL_TIMEOUT)
            .map(|_| ())
    }

    fn input(&self, req: u8, value: u16, index: u16, buf: &mut [u8]) -> rusb::Result<usize> {
        self.handle
            .read_control(vendor(Direction::In), req, value, index, buf, CTRL_TIMEOUT)
    }

    pub fn eeprom(&self, addr: u8) -> rusb::Result<u8> {
        let mut b = [0u8; 1];
        match self.input(REQ_EEPROM, 0, addr as u16, &mut b)? {
            1 => Ok(b[0]),
            _ => Err(rusb::Error::Io),
        }
    }

    fn eeprom_range<const N: usize>(&self, start: u8) -> rusb::Result<[u8; N]> {
        let mut out = [0u8; N];
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.eeprom(start + i as u8)?;
        }
        Ok(out)
    }

    /// Trình tự mở y như plugin Windows: start, sync, GPIO, đọc thông số EEPROM, đặt LED +
    /// gain + phơi sáng, rồi bật cảm biến.
    fn start(&mut self) -> rusb::Result<()> {
        self.out(REQ_START, 0, 0, &[])?;
        self.out(REQ_SYNC, 0, 0, &[0u8; 16])?;
        let mut ack = [0u8; 4];
        let _ = self.handle.read_bulk(EP_IMAGE, &mut ack, Duration::from_millis(500));
        let mut gpio = [0u8; 2];
        let _ = self.input(REQ_GPIO_GET, 0, 0x55, &mut gpio);

        let cam: [u8; 12] = self.eeprom_range(0x30)?;
        let leds: [u8; 6] = self.eeprom_range(0x70)?;
        let (settings, from_eeprom) = CaptureSettings::from_eeprom(&cam, &leds);
        self.settings = settings;
        self.settings_from_eeprom = from_eeprom;
        // Không bắt buộc: chỉ để trả tham số 1104 giống SDK gốc.
        if let Ok(c) = self.eeprom_range::<16>(0xe8) {
            self.custom = c;
        }

        self.apply_settings()?;
        for (idx, val) in [(0x53u16, 1u16), (0x32, 0), (0x31, 1), (0x30, 3)] {
            self.out(REQ_GPIO_SET, val, idx, &[])?;
        }
        self.started = true;
        Ok(())
    }

    fn apply_settings(&self) -> rusb::Result<()> {
        let s = self.settings;
        for (i, &level) in s.leds.iter().enumerate() {
            self.out(REQ_GPIO_SET, ((level as u16) << 8) | 1, i as u16, &[])?;
        }
        for (regs, val) in [
            ([0xa5u16, 0xa6], s.gain_red),
            ([0xa3, 0xa4], s.gain_green1),
            ([0xa9, 0xaa], s.gain_green2),
            ([0xa7, 0xa8], s.gain_blue),
        ] {
            for r in regs {
                self.out(REQ_SENSOR_REG, val, r, &[])?;
            }
        }
        self.out(REQ_SENSOR_REG, s.exposure >> 8, 3, &[])?;
        self.out(REQ_SENSOR_REG, s.exposure & 0xff, 4, &[])
    }

    pub fn finger_present(&self) -> rusb::Result<bool> {
        let mut b = [0u8; 1];
        Ok(self.input(REQ_FINGER, 0, 0, &mut b)? == 1 && b[0] == 1)
    }

    /// Một lần thử chụp, KHÔNG chặn chờ: chưa có ngón tay -> Ok(None) ngay (libzkfp tự gọi lại
    /// theo vòng lặp, giống plugin Windows hỏi 0xEA khoảng 4 lần/giây).
    pub fn capture(&mut self) -> rusb::Result<Option<&[u8]>> {
        if !self.finger_present()? {
            return Ok(None);
        }
        let total = self.image.len();
        let mut got = 0;
        while got < total {
            match self.handle.read_bulk(EP_IMAGE, &mut self.image[got..], IMAGE_TIMEOUT) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if got == 0 => return Err(e),
                Err(_) => break,
            }
        }
        if got < total {
            return Ok(None); // ảnh thiếu: bỏ, lần sau chụp lại
        }
        Ok(Some(&self.image))
    }

    /// Hỏi-đáp bản quyền: thuật toán zkfinger10 gọi ngược qua sensorCheckLic; firmware trả 4 byte
    /// (tất định theo challenge — đã kiểm chứng trên ZK9500).
    pub fn license_response(&self, challenge: u32) -> rusb::Result<u32> {
        let mut b = [0u8; 4];
        let n = self.input(
            REQ_LICENSE,
            (challenge >> 16) as u16,
            (challenge & 0xffff) as u16,
            &mut b,
        )?;
        if n != 4 {
            return Err(rusb::Error::Io);
        }
        Ok(u32::from_le_bytes(b))
    }

    fn stop(&mut self) {
        if self.started {
            for idx in [0x15u16, 0x30, 0x31] {
                let _ = self.out(REQ_GPIO_SET, 0, idx, &[]);
            }
            self.started = false;
        }
        let _ = self.handle.release_interface(0);
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Liệt kê đầu đọc được hỗ trợ, theo thứ tự ổn định (bus, địa chỉ) để index không nhảy lung tung.
pub fn enumerate(ctx: &Context) -> rusb::Result<Vec<rusb::Device<Context>>> {
    let mut found: Vec<_> = ctx
        .devices()?
        .iter()
        .filter(|d| {
            d.device_descriptor()
                .map(|x| x.vendor_id() == VID && SUPPORTED_PIDS.contains(&x.product_id()))
                .unwrap_or(false)
        })
        .collect();
    found.sort_by_key(|d| (d.bus_number(), d.address()));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Byte EEPROM thật của đầu đọc ZK9500 SN 1985252300666 (từ pcap).
    const CAM: [u8; 12] = [0xb0, 0x04, 0xc8, 0x00, 0xc8, 0x00, 0xb6, 0x00, 0xc8, 0x00, 0xbf, 0x07];
    const LEDS: [u8; 6] = [0xc8, 0xc8, 0xaf, 0xaf, 0x78, 0x78];

    #[test]
    fn eeprom_of_reference_reader_matches_windows_log() {
        // Windows: "CMOS Sensor->Exposure:1200, RedGain:200, GreenGain1:200, GreenGain2:182,
        // BlueGain:200. Main1LED:200, Main2LED:200, Side1LED:175, Side2LED:175, Anti1LED:120..."
        let (s, ok) = CaptureSettings::from_eeprom(&CAM, &LEDS);
        assert!(ok);
        assert_eq!(s, CaptureSettings::default());
    }

    #[test]
    fn corrupt_eeprom_falls_back_to_defaults() {
        let mut cam = CAM;
        cam[10] = 0x00; // hỏng checksum
        let (s, ok) = CaptureSettings::from_eeprom(&cam, &LEDS);
        assert!(!ok);
        assert_eq!(s, CaptureSettings::default());
        let (_, ok) = CaptureSettings::from_eeprom(&[0xff; 12], &[0xff; 6]);
        assert!(!ok);
    }
}

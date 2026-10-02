//! libsilkidcap.so thay thế cho ZKFinger SDK 5.3 Linux (ZK10.0), điều khiển đầu đọc ZK9500.
//!
//! libzkfp.so của SDK nạp "libsilkidcap.so" theo tên (DT_NEEDED) và chỉ gọi 10 hàm sensor*
//! bên dưới. Bản gốc chỉ nhận cứng 1b55:0120 và nói giao thức của cảm biến đó; với ZK9500
//! (1b55:0124) nó đọc EEPROM ra 0 byte rồi tự đóng thiết bị. Thư viện này giữ NGUYÊN ABI
//! (chữ ký, mã tham số, mã lỗi — lấy từ objdump của bản gốc) nhưng nói giao thức ZK9500
//! (src/device.rs). libzkfp.so + libzkfinger10.so (thuật toán) giữ nguyên nên mẫu vân tay vẫn
//! là chuẩn ZK10, tương thích với mẫu tạo từ Windows.

pub mod device;

use device::Device;
use rusb::Context;
use std::ffi::{c_int, c_uint, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

// Mã lỗi giống bản gốc.
const ERR_INVALID_HANDLE: c_int = -2;
const ERR_BUFFER_TOO_SMALL: c_int = -3;
const ERR_NOT_SUPPORTED: c_int = -5;
const ERR_NO_DEVICE: c_int = -19;
const ERR_USB: c_int = -1;

const REOPEN_INTERVAL: Duration = Duration::from_secs(2);

fn debug_enabled() -> bool {
    std::env::var_os("ZK9500CAP_DEBUG").is_some()
}

macro_rules! dbg_log {
    ($($t:tt)*) => {
        if debug_enabled() { eprintln!("[zk9500cap] {}", format!($($t)*)); }
    };
}

/// Mọi hàm xuất ra C đều đi qua đây: panic không được phép vượt ranh giới FFI.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ── Trạng thái toàn cục ─────────────────────────────────────────────────────────────

struct Global {
    ctx: Option<Context>,
    /// Địa chỉ các handle còn sống. libzkfp.so giữ g_hDevice SAU khi CloseDevice và thuật
    /// toán vẫn có thể gọi sensorCheckLic trên nó -> chỉ deref con trỏ có trong danh sách này.
    live: Vec<usize>,
}

static GLOBAL: Mutex<Global> = Mutex::new(Global { ctx: None, live: Vec::new() });

fn context(g: &mut Global) -> Option<Context> {
    if g.ctx.is_none() {
        match Context::new() {
            Ok(c) => g.ctx = Some(c),
            Err(e) => {
                dbg_log!("libusb init lỗi: {e}");
                return None;
            }
        }
    }
    g.ctx.clone()
}

fn device_count() -> c_int {
    let ctx = match context(&mut lock(&GLOBAL)) {
        Some(c) => c,
        None => return 0,
    };
    device::enumerate(&ctx).map(|v| v.len() as c_int).unwrap_or(0)
}

// ── Handle trả cho libzkfp.so ───────────────────────────────────────────────────────

/// Thông tin cố định của đầu đọc, giữ lại để vẫn trả lời được tham số khi đang mở lại.
#[derive(Clone)]
struct Info {
    vid: u16,
    pid: u16,
    width: u16,
    height: u16,
    dpi: u32,
    manufacturer: String,
    model: String,
    serial: String,
    custom: [u8; 16],
}

struct Inner {
    dev: Option<Device>,
    last_reopen: Instant,
}

pub struct Sensor {
    info: Info,
    inner: Mutex<Inner>,
}

impl Sensor {
    /// Rút/cắm lại đầu đọc: mở lại đúng thiết bị (theo serial), tối đa mỗi 2 giây một lần.
    fn reopen(&self, inner: &mut Inner) {
        if inner.last_reopen.elapsed() < REOPEN_INTERVAL {
            return;
        }
        inner.last_reopen = Instant::now();
        inner.dev = None;
        let ctx = match context(&mut lock(&GLOBAL)) {
            Some(c) => c,
            None => return,
        };
        let Ok(list) = device::enumerate(&ctx) else { return };
        for d in list {
            match Device::open(&d) {
                Ok(dev) if dev.serial == self.info.serial || self.info.serial.is_empty() => {
                    dbg_log!("đã mở lại đầu đọc {}", dev.serial);
                    inner.dev = Some(dev);
                    return;
                }
                _ => {}
            }
        }
    }
}

/// Chỉ trả về &Sensor nếu con trỏ đúng là handle còn sống do sensorOpen cấp.
fn sensor<'a>(h: *mut c_void) -> Option<&'a Sensor> {
    if h.is_null() || !lock(&GLOBAL).live.contains(&(h as usize)) {
        return None;
    }
    // SAFETY: địa chỉ nằm trong `live` <=> Box<Sensor> chưa bị sensorClose giải phóng.
    Some(unsafe { &*(h as *const Sensor) })
}

// ── 10 hàm mà libzkfp.so nhập từ libsilkidcap.so ────────────────────────────────────

/// 0 nếu có ít nhất một đầu đọc; -19 nếu không (libzkfp đổi thành ZKFP_ERR_INIT = -2).
#[no_mangle]
pub extern "C" fn sensorInit() -> c_int {
    guard(ERR_NO_DEVICE, || {
        let n = device_count();
        dbg_log!("sensorInit: {n} đầu đọc ZK9500");
        if n > 0 { 0 } else { ERR_NO_DEVICE }
    })
}

#[no_mangle]
pub extern "C" fn sensorGetCount() -> c_int {
    guard(0, device_count)
}

/// Giải phóng ngữ cảnh libusb nếu không còn handle nào mở (ZKFPM_Terminate).
#[no_mangle]
pub extern "C" fn sensorFree() -> c_int {
    guard(0, || {
        let mut g = lock(&GLOBAL);
        if g.live.is_empty() {
            g.ctx = None;
        }
        0
    })
}

#[no_mangle]
pub extern "C" fn sensorOpen(index: c_int) -> *mut c_void {
    guard(std::ptr::null_mut(), || {
        let ctx = match context(&mut lock(&GLOBAL)) {
            Some(c) => c,
            None => return std::ptr::null_mut(),
        };
        let list = match device::enumerate(&ctx) {
            Ok(l) => l,
            Err(e) => {
                dbg_log!("liệt kê USB lỗi: {e}");
                return std::ptr::null_mut();
            }
        };
        let Some(d) = usize::try_from(index).ok().and_then(|i| list.get(i)) else {
            dbg_log!("sensorOpen({index}): chỉ có {} đầu đọc", list.len());
            return std::ptr::null_mut();
        };
        let dev = match Device::open(d) {
            Ok(dev) => dev,
            Err(e) => {
                dbg_log!("sensorOpen({index}) lỗi: {e}");
                return std::ptr::null_mut();
            }
        };
        dbg_log!(
            "mở {} {} SN {} ({}x{} @{}dpi, thông số {})",
            dev.manufacturer,
            dev.model,
            dev.serial,
            dev.width,
            dev.height,
            dev.dpi,
            if dev.settings_from_eeprom { "từ EEPROM" } else { "MẶC ĐỊNH (EEPROM lạ)" }
        );
        let info = Info {
            vid: dev.vid,
            pid: dev.pid,
            width: dev.width,
            height: dev.height,
            dpi: dev.dpi,
            manufacturer: dev.manufacturer.clone(),
            model: dev.model.clone(),
            serial: dev.serial.clone(),
            custom: dev.custom,
        };
        let s = Box::new(Sensor {
            info,
            inner: Mutex::new(Inner { dev: Some(dev), last_reopen: Instant::now() }),
        });
        let p = Box::into_raw(s) as *mut c_void;
        lock(&GLOBAL).live.push(p as usize);
        p
    })
}

#[no_mangle]
pub extern "C" fn sensorClose(h: *mut c_void) -> c_int {
    guard(ERR_INVALID_HANDLE, || {
        let mut g = lock(&GLOBAL);
        let Some(pos) = g.live.iter().position(|&p| p == h as usize) else {
            return ERR_INVALID_HANDLE;
        };
        g.live.swap_remove(pos);
        drop(g);
        // SAFETY: vừa gỡ khỏi `live` nên không ai khác còn lấy được con trỏ này.
        drop(unsafe { Box::from_raw(h as *mut Sensor) });
        0
    })
}

/// Một lần thử chụp, không chặn. Trả số byte ảnh (rộng x cao) khi có ngón tay, 0 khi chưa có,
/// âm khi lỗi. Tham số thứ 3 bị bỏ qua có chủ đích: CZKFPHelper::AcquireTemplate KHÔNG nạp
/// thanh ghi edx trước khi gọi (giá trị rác) — bản gốc cũng luôn chép rộng x cao byte.
#[no_mangle]
pub extern "C" fn sensorCapture(h: *mut c_void, buf: *mut u8, _size: c_int) -> c_int {
    guard(ERR_USB, || {
        let Some(s) = sensor(h) else { return ERR_INVALID_HANDLE };
        if buf.is_null() {
            return ERR_INVALID_HANDLE;
        }
        let mut inner = lock(&s.inner);
        if inner.dev.is_none() {
            s.reopen(&mut inner);
        }
        let Some(dev) = inner.dev.as_mut() else { return ERR_NO_DEVICE };
        match dev.capture() {
            Ok(Some(img)) => {
                // SAFETY: libzkfp cấp buffer rộng x cao (kiểm tra ở AcquireTemplateImage /
                // ZKFPM_AcquireFingerprint), đúng bằng img.len().
                unsafe { std::ptr::copy_nonoverlapping(img.as_ptr(), buf, img.len()) };
                img.len() as c_int
            }
            Ok(None) => 0,
            Err(e) => {
                dbg_log!("chụp lỗi: {e} -> sẽ mở lại đầu đọc");
                inner.dev = None;
                s.reopen(&mut inner);
                if matches!(e, rusb::Error::NoDevice) { ERR_NO_DEVICE } else { ERR_USB }
            }
        }
    })
}

fn int_param(s: &Sensor, code: c_int) -> c_int {
    let i = &s.info;
    match code {
        1 => i.width as c_int,
        2 => i.height as c_int,
        3 => i.dpi as c_int,
        4 => 7,
        5 | 6 | 107 | 200 | 201 | 1016 | 2001..=2003 => 0,
        106 => i.width as c_int * i.height as c_int,
        1015 => (i.vid as u32 | (i.pid as u32) << 16) as c_int,
        2004 => -1,
        _ => ERR_NOT_SUPPORTED,
    }
}

/// Mã: 1 rộng, 2 cao, 3 dpi, 106 rộng x cao, 1015 VID|PID<<16 ...
#[no_mangle]
pub extern "C" fn sensorGetParameter(h: *mut c_void, code: c_int) -> c_int {
    guard(ERR_USB, || match sensor(h) {
        Some(s) => int_param(s, code),
        None => ERR_INVALID_HANDLE,
    })
}

fn is_int_code(code: c_int) -> bool {
    matches!(code, 1..=7 | 101..=107 | 200..=201 | 1002..=1016 | 2001..=2004)
}

/// ZKFPM_GetParameters đi thẳng xuống đây. Số nguyên: 4 byte LE. Chuỗi: 1101 hãng, 1102 model,
/// 1103 serial (không kèm NUL trong *size, như bản gốc), 1104 = 16 byte custom SN.
#[no_mangle]
pub extern "C" fn sensorGetParameterEx(
    h: *mut c_void,
    code: c_int,
    buf: *mut u8,
    size: *mut c_int,
) -> c_int {
    guard(ERR_USB, || {
        let Some(s) = sensor(h) else { return ERR_INVALID_HANDLE };
        if buf.is_null() || size.is_null() {
            return ERR_INVALID_HANDLE;
        }
        // SAFETY: con trỏ do ZKFPM_GetParameters của người gọi cấp, *size là dung lượng buf.
        let cap = unsafe { *size };
        let write = |bytes: &[u8], reported: usize| -> c_int {
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
                *size = reported as c_int;
            }
            0
        };
        if is_int_code(code) {
            if cap < 4 {
                return ERR_BUFFER_TOO_SMALL;
            }
            return write(&int_param(s, code).to_le_bytes(), 4);
        }
        let text = match code {
            1101 => &s.info.manufacturer,
            1102 => &s.info.model,
            1103 => &s.info.serial,
            1104 => {
                if cap < 16 {
                    return ERR_BUFFER_TOO_SMALL;
                }
                return write(&s.info.custom, 16);
            }
            _ => return ERR_NOT_SUPPORTED,
        };
        let len = text.len();
        if (cap as usize) < len || cap < 0 {
            return ERR_BUFFER_TOO_SMALL;
        }
        if (cap as usize) > len {
            let mut z = text.as_bytes().to_vec();
            z.push(0);
            write(&z, len)
        } else {
            write(text.as_bytes(), len)
        }
    })
}

/// Tham số ghi (đèn/còi/...) chưa có ý nghĩa trên ZK9500 -> nhận và bỏ qua để
/// ZKFPM_SetParameters không báo lỗi.
#[no_mangle]
pub extern "C" fn sensorSetParameter(h: *mut c_void, code: c_int, value: c_int) -> c_int {
    guard(ERR_USB, || {
        if sensor(h).is_none() {
            return ERR_INVALID_HANDLE;
        }
        dbg_log!("sensorSetParameter({code}, {value}) bỏ qua");
        0
    })
}

#[no_mangle]
pub extern "C" fn sensorSetParameterEx(
    h: *mut c_void,
    code: c_int,
    buf: *const u8,
    size: c_int,
) -> c_int {
    guard(ERR_USB, || {
        if h.is_null() || buf.is_null() {
            return ERR_INVALID_HANDLE;
        }
        if !is_int_code(code) || code == 1015 || code == 1016 {
            return ERR_NOT_SUPPORTED;
        }
        if size < 4 {
            return ERR_BUFFER_TOO_SMALL;
        }
        // SAFETY: size >= 4 byte do người gọi cấp.
        let value = unsafe { std::ptr::read_unaligned(buf as *const c_int) };
        sensorSetParameter(h, code, value)
    })
}

/// Callback bản quyền của thuật toán (libzkfp: BIOKEY_SET_CHECK_CALLBACK -> CheckValue ->
/// sensorCheckLic(g_hDevice, challenge, ptr)). Trả nguyên 4 byte firmware trả lời.
#[no_mangle]
pub extern "C" fn sensorCheckLic(h: *mut c_void, challenge: c_uint, _ptr: *mut c_void) -> c_int {
    guard(ERR_USB, || {
        let Some(s) = sensor(h) else { return ERR_INVALID_HANDLE };
        let mut inner = lock(&s.inner);
        if inner.dev.is_none() {
            s.reopen(&mut inner);
        }
        let Some(dev) = inner.dev.as_ref() else { return ERR_NO_DEVICE };
        match dev.license_response(challenge) {
            Ok(v) => v as c_int,
            Err(e) => {
                dbg_log!("sensorCheckLic({challenge:#x}) lỗi: {e}");
                ERR_USB
            }
        }
    })
}

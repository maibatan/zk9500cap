//! Kiểm tra nhanh đầu đọc mà không cần SDK: liệt kê, mở, in thông số, chờ ngón tay và lưu ảnh PGM.
//!     zkprobe            # chờ tối đa 30 giây, chụp 1 ảnh -> zk9500-1.pgm
//!     zkprobe 3 60       # chụp 3 ảnh, chờ tối đa 60 giây
use rusb::{Context, UsbContext};
use silkidcap::device::{self, Device};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let want: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let ctx = Context::new().expect("libusb init");
    let all = ctx.devices().expect("liệt kê USB");
    for d in all.iter() {
        if let Ok(x) = d.device_descriptor() {
            if x.vendor_id() == device::VID {
                println!(
                    "thấy {:04x}:{:04x} bus {} addr {}{}",
                    x.vendor_id(),
                    x.product_id(),
                    d.bus_number(),
                    d.address(),
                    if device::SUPPORTED_PIDS.contains(&x.product_id()) { "" } else { " (CHƯA hỗ trợ)" }
                );
            }
        }
    }
    let list = device::enumerate(&ctx).expect("liệt kê USB");
    let Some(d) = list.first() else {
        eprintln!("Không có đầu đọc ZK9500 (1b55:0124). Kiểm tra 'lsusb' / quyền /dev/bus/usb.");
        std::process::exit(2);
    };
    let mut dev = match Device::open(d) {
        Ok(dev) => dev,
        Err(e) => {
            eprintln!("Mở đầu đọc lỗi: {e} (thiếu quyền? thử sudo hoặc udev rule cho 1b55)");
            std::process::exit(3);
        }
    };
    println!(
        "Mở OK: {} {} SN {} rev {:04x}, ảnh {}x{} @{}dpi",
        dev.manufacturer, dev.model, dev.serial, dev.rev, dev.width, dev.height, dev.dpi
    );
    println!(
        "Thông số ({}): {:?}",
        if dev.settings_from_eeprom { "từ EEPROM" } else { "MẶC ĐỊNH" },
        dev.settings
    );
    match dev.license_response(0x12345678) {
        Ok(v) => println!("Hỏi-đáp bản quyền 0x12345678 -> {v:#010x}"),
        Err(e) => println!("Hỏi-đáp bản quyền LỖI: {e}"),
    }

    println!("Đặt ngón tay lên đầu đọc (chờ {secs} giây)...");
    let (w, h) = (dev.width as usize, dev.height as usize);
    let end = Instant::now() + Duration::from_secs(secs);
    let mut n = 0;
    while n < want && Instant::now() < end {
        let t = Instant::now();
        match dev.capture() {
            Ok(Some(img)) => {
                n += 1;
                let mean = img.iter().map(|&b| b as u64).sum::<u64>() / img.len() as u64;
                let name = format!("zk9500-{n}.pgm");
                let mut pgm = format!("P5\n{w} {h}\n255\n").into_bytes();
                pgm.extend_from_slice(img);
                std::fs::write(&name, pgm).expect("ghi ảnh");
                println!("Chụp #{n}: {} byte trong {} ms, độ sáng TB {mean} -> {name}", img.len(), t.elapsed().as_millis());
                std::thread::sleep(Duration::from_millis(800));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => {
                eprintln!("Chụp lỗi: {e}");
                std::process::exit(4);
            }
        }
    }
    println!("Xong: {n}/{want} ảnh.");
    if n < want {
        std::process::exit(5);
    }
}

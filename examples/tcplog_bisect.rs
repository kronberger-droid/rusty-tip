//! Does command traffic on the programming interface stop a running TCP
//! logger stream? Point it at a Nanonis host, watch the frame counts.
//!
//! Under Wine every step after "baseline" reports 0, including commands sent
//! on a second connection. Run this against a Windows host to tell whether
//! that is a Wine limitation or how Nanonis behaves everywhere.
use nanonis_rs::NanonisClient;
use std::io::Read;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = NanonisClient::new("127.0.0.1", 6501)?;   // starts the logger
    let mut b = NanonisClient::new("127.0.0.1", 6502)?;   // separate connection

    let _ = a.tcplog_stop();
    std::thread::sleep(Duration::from_millis(400));
    a.tcplog_chs_set(vec![0, 1, 2])?;
    a.tcplog_oversampl_set(2)?;
    let mut sock = TcpStream::connect("127.0.0.1:6590")?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;
    std::thread::sleep(Duration::from_millis(600));
    a.tcplog_start()?;

    let count = Arc::new(AtomicU64::new(0));
    let c2 = count.clone();
    std::thread::spawn(move || {
        let mut hdr = [0u8; 18];
        loop {
            if sock.read_exact(&mut hdr).is_err() { return; }
            let nch = u32::from_be_bytes(hdr[0..4].try_into().unwrap());
            let mut d = vec![0u8; nch as usize * 4];
            if sock.read_exact(&mut d).is_err() { return; }
            c2.fetch_add(1, Ordering::Relaxed);
        }
    });

    let step = |label: &str, f: &mut dyn FnMut()| {
        let before = count.load(Ordering::Relaxed);
        f();
        std::thread::sleep(Duration::from_millis(1500));
        println!("{label:<34} frames: {}", count.load(Ordering::Relaxed) - before);
    };

    step("baseline", &mut || {});
    step("bias_set on B(6502)", &mut || { let _ = b.bias_set(0.4); });
    step("still alive on B?", &mut || {});
    step("rt_freq_get on B(6502)", &mut || { let _ = b.util_rt_freq_get(); });
    step("read-only rt_freq_get on A(6501)", &mut || { let _ = a.util_rt_freq_get(); });
    step("after", &mut || {});
    Ok(())
}

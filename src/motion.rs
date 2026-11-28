use anyhow::{Context, Result, ensure};
use libloading::os::windows::Library;
use std::{
    collections::HashMap,
    ffi::c_void,
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

const VERSION: u32 = 0x100000;
const INFO: u32 = 0x100001;
const DATA: u32 = 0x100002;
type Input = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Motion {
    orientation: [f32; 4],
    accel: [f32; 3],
    gyro: [f32; 3],
}

// Method order is part of the versioned SteamClient023 and SteamInput006 interfaces.
macro_rules! method {
    ($object:expr, $index:expr, $signature:ty) => {
        std::mem::transmute::<*const c_void, $signature>(crate::steam::vfn($object, $index))
    };
}

struct SteamInput {
    client: Input,
    pipe: i32,
    user: i32,
    input: Input,
    _library: Library,
}

impl SteamInput {
    fn open() -> Result<Self> {
        let steam = crate::steam::SteamInstallation::detect()?;
        let path = steam.steam_path.join("steamclient64.dll").canonicalize()?;
        unsafe {
            let library = Library::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
            .with_context(|| format!("Load {}", path.display()))?;
            let create = library
                .get::<unsafe extern "C" fn(*const i8, *mut i32) -> Input>(b"CreateInterface\0")?;
            let client = create(c"SteamClient023".as_ptr(), std::ptr::null_mut());
            ensure!(!client.is_null(), "SteamClient023 unavailable");
            let pipe = method!(client, 0, unsafe extern "C" fn(Input) -> i32)(client);
            ensure!(pipe != 0, "Could not open a Steam client pipe");
            let mut api = Self {
                client,
                pipe,
                user: 0,
                input: std::ptr::null_mut(),
                _library: library,
            };
            api.user = method!(client, 2, unsafe extern "C" fn(Input, i32) -> i32)(client, pipe);
            ensure!(api.user != 0, "Sign in to Steam to forward motion");
            let get_interface = method!(
                client,
                12,
                unsafe extern "C" fn(Input, i32, i32, *const i8) -> Input
            );
            let input = get_interface(client, api.user, pipe, c"SteamInput006".as_ptr());
            ensure!(!input.is_null(), "SteamInput006 unavailable");
            ensure!(
                method!(input, 0, unsafe extern "C" fn(Input, bool) -> bool)(input, true),
                "Steam Input initialization failed"
            );
            api.input = input;
            Ok(api)
        }
    }

    fn sample(&self, enabled: bool) -> [Option<Motion>; 4] {
        let input = self.input;
        unsafe {
            method!(input, 3, unsafe extern "C" fn(Input, bool))(input, true);
            let handle = method!(input, 38, unsafe extern "C" fn(Input, i32) -> u64);
            let session = method!(input, 45, unsafe extern "C" fn(Input, u64) -> u32);
            // Win64 C++ returns this 40-byte struct in a caller-owned buffer after the this pointer.
            let motion = method!(
                input,
                29,
                unsafe extern "C" fn(Input, *mut Motion, u64) -> *mut Motion
            );
            std::array::from_fn(|slot| {
                if !enabled {
                    return None;
                }
                let pad = handle(input, slot as i32);
                (pad != 0 && session(input, pad) != 0).then(|| {
                    let mut sample = std::mem::MaybeUninit::uninit();
                    motion(input, sample.as_mut_ptr(), pad);
                    sample.assume_init()
                })
            })
        }
    }
}

impl Drop for SteamInput {
    fn drop(&mut self) {
        unsafe {
            if !self.input.is_null() {
                method!(self.input, 1, unsafe extern "C" fn(Input) -> bool)(self.input);
            }
            if self.user != 0 {
                method!(self.client, 4, unsafe extern "C" fn(Input, i32, i32))(
                    self.client,
                    self.pipe,
                    self.user,
                );
            }
            method!(self.client, 1, unsafe extern "C" fn(Input, i32) -> bool)(
                self.client,
                self.pipe,
            );
        }
    }
}

pub struct Bridge {
    socket: UdpSocket,
    steam: SteamInput,
    clients: HashMap<(SocketAddr, usize), Instant>,
    start: Instant,
    last: Option<Instant>,
    sequence: u32,
}

fn info(slot: usize, active: bool) -> [u8; 12] {
    [
        slot as u8,
        if active { 2 } else { 0 },
        2,
        2,
        2,
        0,
        0,
        0,
        0,
        slot as u8 + 1,
        0xef,
        active as u8,
    ]
}

fn packet(kind: u32, payload: &[u8], id: u32) -> Vec<u8> {
    let mut out = b"DSUS".to_vec();
    out.extend(1001u16.to_le_bytes());
    out.extend(((payload.len() + 4) as u16).to_le_bytes());
    out.extend([0; 4]);
    out.extend(id.to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend(payload);
    let crc = crc32fast::hash(&out);
    out[8..12].copy_from_slice(&crc.to_le_bytes());
    out
}

fn request(msg: &mut [u8]) -> Option<(u32, &[u8])> {
    if msg.len() < 20
        || &msg[..4] != b"DSUC"
        || msg[4..6] != 1001u16.to_le_bytes()
        || usize::from(u16::from_le_bytes([msg[6], msg[7]])) + 16 != msg.len()
    {
        return None;
    }
    let crc = u32::from_le_bytes(msg[8..12].try_into().ok()?);
    msg[8..12].fill(0);
    (crc32fast::hash(msg) == crc).then(|| {
        (
            u32::from_le_bytes(msg[16..20].try_into().unwrap()),
            &msg[20..],
        )
    })
}

fn data(slot: usize, m: Motion, seq: u32, us: u64) -> [u8; 80] {
    let mut out = [0; 80];
    out[..12].copy_from_slice(&info(slot, true));
    out[12..16].copy_from_slice(&seq.to_le_bytes());
    out[20..24].fill(128);
    out[48..56].copy_from_slice(&us.to_le_bytes());
    let [ax, ay, az] = m.accel;
    let [gx, gy, gz] = m.gyro;
    let axes = [-ax, -az, ay]
        .map(|x| x / 16384.)
        .into_iter()
        .chain([gx, -gz, -gy].map(|x| x * 2000. / 32768.));
    for (dest, v) in out[56..].as_chunks_mut::<4>().0.iter_mut().zip(axes) {
        dest.copy_from_slice(&v.to_le_bytes())
    }
    out
}

pub fn endpoint(ip: &str, port: u16) -> Result<std::net::SocketAddrV4> {
    let ip: std::net::Ipv4Addr = ip.trim().parse()?;
    ensure!(
        ip.is_loopback() && port > 0,
        "Use a loopback IPv4 address and nonzero port"
    );
    Ok(std::net::SocketAddrV4::new(ip, port))
}

impl Bridge {
    pub fn open(address: &str) -> Result<Self> {
        let (ip, port) = address.rsplit_once(':').context("Missing UDP port")?;
        let addr = endpoint(ip, port.parse()?)?;
        let socket = UdpSocket::bind(addr).with_context(|| format!("Bind {addr}"))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            steam: SteamInput::open()?,
            clients: HashMap::new(),
            start: Instant::now(),
            last: None,
            sequence: 0,
        })
    }

    pub fn tick(&mut self, enabled: bool) -> Result<()> {
        let now = Instant::now();
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(8))
        {
            return Ok(());
        }
        self.last = Some(now);
        let pads = self.steam.sample(enabled);
        let id = std::process::id();
        let mut buf = [0u8; 2048];
        for _ in 0..32 {
            let (n, addr) = match self.socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e)
                    if e.kind() == ErrorKind::ConnectionReset
                        || e.raw_os_error() == Some(10040) =>
                {
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let Some((kind, p)) = request(&mut buf[..n]) else {
                continue;
            };
            match kind {
                VERSION => {
                    let _ = self
                        .socket
                        .send_to(&packet(VERSION, &1001u16.to_le_bytes(), id), addr);
                }
                INFO if p.len() >= 4 => {
                    let count = u32::from_le_bytes(p[..4].try_into()?) as usize;
                    if count > 4 || p.len() < 4 + count {
                        continue;
                    }
                    for &slot in &p[4..4 + count] {
                        if let Some(pad) = pads.get(slot as usize) {
                            let mut meta = info(slot as usize, pad.is_some());
                            meta[11] = 0;
                            let _ = self.socket.send_to(&packet(INFO, &meta, id), addr);
                        }
                    }
                }
                DATA if p.len() == 8 => {
                    for slot in 0..4 {
                        if p[0] == 0
                            || (p[0] & 1 != 0 && p[1] as usize == slot)
                            || (p[0] & 2 != 0 && p[2..] == info(slot, true)[4..10])
                        {
                            self.clients.insert((addr, slot), now);
                        }
                    }
                }
                _ => {}
            }
        }
        self.clients
            .retain(|_, last| now.duration_since(*last) < Duration::from_secs(5));
        for &(addr, slot) in self.clients.keys() {
            if let Some(m) = pads[slot] {
                let payload = data(
                    slot,
                    m,
                    self.sequence,
                    self.start.elapsed().as_micros() as u64,
                );
                let _ = self.socket.send_to(&packet(DATA, &payload, id), addr);
            }
        }
        self.sequence = self.sequence.wrapping_add(1);
        Ok(())
    }
}

pub struct Worker {
    child: std::process::Child,
    enabled: bool,
}

impl Worker {
    pub fn start(appid: u32, address: &str, enabled: bool) -> Result<Self> {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        let child = Command::new(std::env::current_exe()?)
            .args(["--motion-worker", address])
            .env("SteamAppId", appid.to_string())
            .env("SteamGameId", appid.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()?;
        let mut worker = Self {
            child,
            enabled: !enabled,
        };
        worker.update(enabled)?;
        Ok(worker)
    }
    pub fn update(&mut self, enabled: bool) -> Result<()> {
        use std::io::Write;
        if let Some(exit) = self.child.try_wait()? {
            anyhow::bail!("Motion helper exited ({exit}). See launch.log")
        }
        if self.enabled != enabled {
            self.child
                .stdin
                .as_mut()
                .context("Motion helper closed")?
                .write_all(&[u8::from(enabled)])?;
            self.enabled = enabled;
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn worker(address: &str) -> Result<()> {
    use std::io::Read;
    let mut bridge = Bridge::open(address)
        .inspect_err(|e| crate::runner::log(&format!("Motion startup: {e:#}")))?;
    crate::runner::log(&format!("Motion forwarding on {address}"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut byte = [0];
        let mut pipe = std::io::stdin().lock();
        while pipe.read_exact(&mut byte).is_ok() && tx.send(byte[0] != 0).is_ok() {}
    });
    let mut enabled = false;
    loop {
        match rx.try_recv() {
            Ok(value) => enabled = value,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        bridge.tick(enabled)?;
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_packet_uses_dsu_slots_and_units() {
        let sample = Motion {
            orientation: [0.; 4],
            accel: [8192., 4096., -16384.],
            gyro: [8192., 4096., -16384.],
        };
        let bytes = packet(DATA, &data(2, sample, 7, 42), 1);
        assert_eq!(bytes.len(), 100);
        assert_eq!(bytes[20], 2);
        assert_eq!(&bytes[68..76], &42u64.to_le_bytes());
        for (axis, expected) in bytes[76..]
            .as_chunks::<4>()
            .0
            .iter()
            .zip([-0.5_f32, 1., 0.25, 500., 1000., -250.])
        {
            assert_eq!(f32::from_le_bytes(*axis), expected);
        }
        assert!(endpoint("127.0.0.2", 26761).is_ok());
        assert!(endpoint("0.0.0.0", 26760).is_err());
        assert!(endpoint("127.0.0.1", 0).is_err());
    }

    #[test]
    #[ignore = "requires a connected Remote Play guest"]
    fn live_motion_socket() {
        let port = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let addr = format!("127.0.0.1:{port}");
        let mut bridge = Bridge::open(&addr).unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let mut sub = packet(DATA, &[0; 8], 7);
        sub[..4].copy_from_slice(b"DSUC");
        sub[8..12].fill(0);
        let crc = crc32fast::hash(&sub);
        sub[8..12].copy_from_slice(&crc.to_le_bytes());
        client.send_to(&sub, &addr).unwrap();
        let mut got = 0;
        for _ in 0..100 {
            bridge.tick(true).unwrap();
            let mut buf = [0; 128];
            if let Ok(100) = client.recv(&mut buf) {
                assert_eq!(&buf[..4], b"DSUS");
                got += 1;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        println!("Live guest packets received: {got}");
        assert!(got > 0, "no guest motion packets received");
    }
}

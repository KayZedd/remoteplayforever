//! Remote Play Together session host (runs in its own process: `--host-session <appid>`).
//!
//! Attaches to the running Steam client through `steamclient64.dll` and drives its internal
//! `IClientRemoteClientManager`: creates a Remote Play Together group for the given AppID, creates
//! guest invite links, keeps the group published and applies per-guest input permissions.
//! No donor game is involved: the session is bound to whatever AppID the caller passes.
//!
//! The interface is undocumented and its vtable layout shifts between Steam client builds, so
//! methods are never called by hardcoded index. Each client stub references its own name as a
//! string, which lets us build a name -> index map at runtime (`scan_vtable_names`).
//! Technique and call sequence follow RemotePlayForge (MIT, okabeeer) and RemotePlayWhatever.
//!
//! It is a separate process because the group only creates reliably from a fresh process whose
//! calling thread is not an STA (the GUI thread is), and so that a bad call into Steam's
//! internals can only take down this helper, never the GUI.

use crate::runner::log;
use crate::i18n::tr;
use crate::session::{GuestInputs, HostCommand, HostEvent, InputKind};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const CB_REMOTE_PLAY_FIRST: i32 = 1_100_000;
const CB_REMOTE_PLAY_LAST: i32 = 1_100_100;
const CB_INVITE_RESULT: i32 = 1_100_028;
/// Fires repeatedly while a guest is actually streaming.
const CB_STREAM_ACTIVE: i32 = 1_100_042;
/// Fire on every group publish; too frequent to be worth logging.
const CB_GROUP_REFRESH: [i32; 2] = [1_100_035, 1_100_038];
/// Guest joined / session changed: the group has to be re-published to list the guest as a player.
const CB_MEMBERSHIP: [i32; 5] = [1_100_017, 1_100_026, 1_100_032, 1_100_033, 1_100_058];

/// `IClientEngine::GetIClientRemoteClientManager`. The engine object is not an IPC stub, so it
/// carries no name strings; the result is validated by name before any use (see `Host::attach`).
const ENGINE_GET_REMOTE_CLIENT_MANAGER: usize = 56;
const REQUIRED_METHODS: [&str; 4] = [
    "BCreateRemotePlayGroup",
    "GetLocalRemotePlayTogetherGroupID",
    "GetAvailableRemotePlayTogetherGuestID",
    "BCreateRemotePlayInviteAndSession",
];

const INDIVIDUAL_ACCOUNT_HIGH: u64 = 0x0110_0001;
const STEAM_ID_BASE: u64 = 76_561_197_960_265_728;
const MAX_CALLBACK_PAYLOAD: usize = 4096;

#[repr(C)]
struct CallbackMsg {
    user: i32,
    id: i32,
    param: *const u8,
    size: i32,
}

/// `RemotePlayPlayer_t` as the client expects it (40 bytes).
#[repr(C)]
#[derive(Default)]
struct RemotePlayPlayer {
    steam_id: u64,
    guest_id: u32,
    group_id: u32,
    reserved: [u64; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guest {
    pub steam: u64,
    pub guest_id: u32,
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------------------------

fn read_u32(bytes: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(off..off.checked_add(4)?)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(off..off.checked_add(8)?)?.try_into().ok()?))
}

/// Name of the client stub starting at `fn_off` inside a loaded module image: the first
/// `lea reg, [rip+disp32]` whose target is an identifier-looking string inside the image.
pub fn stub_name(image: &[u8], fn_off: usize) -> Option<String> {
    for k in 0..260 {
        let at = fn_off + k;
        let Some(ins) = image.get(at..at + 7) else { break };
        if !matches!(ins[0], 0x48 | 0x4C | 0x49 | 0x4D) || ins[1] != 0x8D || ins[2] & 0xC7 != 0x05 {
            continue;
        }
        let disp = i32::from_le_bytes([ins[3], ins[4], ins[5], ins[6]]) as i64;
        let target = at as i64 + 7 + disp;
        if target < 0 || target as usize >= image.len() {
            continue;
        }
        let tail = &image[target as usize..];
        let raw = &tail[..tail.len().min(96)];
        let raw = &raw[..raw.iter().position(|&c| c == 0).unwrap_or(raw.len())];
        if raw.iter().any(|&c| !(33..=126).contains(&c)) {
            continue;
        }
        let letters = raw.iter().filter(|c| c.is_ascii_alphabetic()).count();
        if raw.len() >= 6 && letters >= 4 {
            return Some(String::from_utf8_lossy(raw).into_owned());
        }
    }
    None
}

/// Map method name -> vtable index for the vtable at `vtable_off` in a module loaded at `base`.
pub fn scan_vtable_names(image: &[u8], base: usize, vtable_off: usize) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for i in 0..400 {
        let Some(func) = read_u64(image, vtable_off + i * 8) else { break };
        let func = func as usize;
        if func < base || func >= base + image.len() {
            break;
        }
        if let Some(name) = stub_name(image, func - base) {
            map.entry(name).or_insert(i);
        }
    }
    map
}

fn looks_like_url(s: &str) -> bool {
    s.len() >= 8 && s.contains('.') && s.contains('/') && !s.contains(' ')
}

fn printable_cstr(bytes: &[u8], off: usize) -> Option<&str> {
    let tail = bytes.get(off..)?;
    let run = &tail[..tail.iter().position(|&c| c == 0).unwrap_or(tail.len())];
    if run.is_empty() || run.iter().any(|&c| !(32..=126).contains(&c)) {
        return None;
    }
    std::str::from_utf8(run).ok()
}

/// Join URL out of a `RemotePlayInviteResult_t` payload. It sits at offset 56 on current builds;
/// if the struct moves, fall back to the first URL-looking string in the payload.
pub fn extract_url(payload: &[u8]) -> Option<String> {
    if let Some(s) = printable_cstr(payload, 56).filter(|s| looks_like_url(s)) {
        return Some(s.to_string());
    }
    (16..payload.len())
        .filter(|&off| off == 0 || payload[off - 1] == 0)
        .filter_map(|off| printable_cstr(payload, off))
        .find(|s| looks_like_url(s) && (s.contains("s.team/") || s.contains("://")))
        .map(str::to_string)
}

/// Steam sometimes returns the link without a scheme ("s.team/..."), which is not clickable.
pub fn normalize_url(url: &str) -> String {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("steam://") {
        u.to_string()
    } else if let Some(rest) = u.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        format!("https://{u}")
    }
}

/// Find guests with a Steam account in a Remote Play callback payload. Such callbacks carry
/// `RemotePlayPlayer_t {steam_id@0, guest_id@8, group_id@12}`; an entry counts only when its group
/// is our live group. Link slots that nobody claimed yet have steam_id 0 and are skipped (they
/// are addressed by guest id instead). The host is never listed.
pub fn scan_guests(payload: &[u8], live_group: u32, host_steam: u64) -> Vec<Guest> {
    let mut found: Vec<Guest> = Vec::new();
    if live_group == 0 {
        return found;
    }
    let mut off = 0;
    while off + 16 <= payload.len() {
        let steam = read_u64(payload, off).unwrap_or(0);
        let guest_id = read_u32(payload, off + 8).unwrap_or(0);
        let group = read_u32(payload, off + 12).unwrap_or(0);
        off += 4;

        let account = steam >> 32 == INDIVIDUAL_ACCOUNT_HIGH && steam as u32 != 0;
        if !account || steam == host_steam || group != live_group || !(1..=64).contains(&guest_id) {
            continue;
        }
        if !found.iter().any(|g| g.steam == steam) {
            found.push(Guest { steam, guest_id });
        }
    }
    found
}

/// Player identities `(steam id, guest id)` that input permissions are written to. A link slot
/// is addressed with steam id 0 until someone with a Steam account claims it; from then on only
/// by that account. Writing both identities makes Steam treat one guest as two players, and
/// their single controller shows up twice on the host.
pub fn input_targets(max_invited_guest: u32, guests: &[Guest]) -> Vec<(u64, u32)> {
    let unclaimed = (1..=max_invited_guest)
        .filter(|id| !guests.iter().any(|g| g.guest_id == *id))
        .map(|id| (0, id));
    unclaimed.chain(guests.iter().map(|g| (g.steam, g.guest_id))).collect()
}

// ---------------------------------------------------------------------------------------------
// Steam client attachment
// ---------------------------------------------------------------------------------------------

type CreateInterfaceFn = unsafe extern "C" fn(*const c_char, *mut i32) -> *mut c_void;
type BGetCallbackFn = unsafe extern "C" fn(i32, *mut CallbackMsg) -> u8;
type FreeLastCallbackFn = unsafe extern "C" fn(i32);

unsafe fn vfn(obj: *mut c_void, index: usize) -> *const c_void {
    unsafe { *(*(obj as *const *const *const c_void)).add(index) }
}

struct Host {
    rcm: *mut c_void,
    names: HashMap<String, usize>,
    pipe: i32,
    bget: BGetCallbackFn,
    bfree: FreeLastCallbackFn,
    appid: u32,
    desktop_stream: bool,
    host_steam: u64,
    guests: Vec<Guest>,
    max_invited_guest: u32,
    want: GuestInputs,
    guest_connected: bool,
    overlay_refreshed: bool,
}

/// Error for a Steam client build whose internals no longer match what this host expects.
fn unsupported(what: &str) -> String {
    format!("{} ({what})", tr("Unsupported Steam client version", "Nieobsługiwana wersja klienta Steam"))
}

fn emit(ev: HostEvent) {
    let line = ev.to_line();
    log(&format!("rpt: {line}"));
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn status(msg: impl Into<String>) {
    emit(HostEvent::Status(msg.into()));
}

impl Host {
    fn attach(appid: u32, desktop_stream: bool, inputs: GuestInputs) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH};

        let steam = crate::steam::SteamInstallation::detect().map_err(|e| e.to_string())?;
        let dll = steam.steam_path.join("steamclient64.dll");
        if !dll.exists() {
            return Err(format!("{} {}", tr("Not found:", "Nie znaleziono"), dll.display()));
        }
        // steamclient64.dll resolves its own dependencies (tier0_s64.dll, ...) relative to the Steam dir.
        let _ = std::env::set_current_dir(&steam.steam_path);
        let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

        unsafe {
            let module = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
            if module.is_null() {
                return Err(tr("Could not load steamclient64.dll", "Nie udało się załadować steamclient64.dll").into());
            }
            let export = |name: &[u8]| GetProcAddress(module, name.as_ptr()).ok_or_else(|| {
                format!("{} {}", tr("steamclient64.dll does not export", "steamclient64.dll nie eksportuje"), String::from_utf8_lossy(&name[..name.len() - 1]))
            });
            let create_interface: CreateInterfaceFn = std::mem::transmute(export(b"CreateInterface\0")?);
            let bget: BGetCallbackFn = std::mem::transmute(export(b"Steam_BGetCallback\0")?);
            let bfree: FreeLastCallbackFn = std::mem::transmute(export(b"Steam_FreeLastCallback\0")?);

            let client = create_interface(c"SteamClient019".as_ptr(), std::ptr::null_mut());
            if client.is_null() {
                return Err(unsupported("SteamClient019"));
            }
            let create_pipe: unsafe extern "C" fn(*mut c_void) -> i32 = std::mem::transmute(vfn(client, 0));
            let connect_user: unsafe extern "C" fn(*mut c_void, i32) -> i32 = std::mem::transmute(vfn(client, 2));
            let pipe = create_pipe(client);
            if pipe == 0 {
                return Err(tr("Steam is not running. Start Steam and sign in.", "Steam nie jest uruchomiony. Uruchom Steam i zaloguj się.").into());
            }
            if connect_user(client, pipe) == 0 {
                return Err(tr("Nobody is signed in to Steam. Sign in and try again.", "Brak zalogowanego użytkownika Steam. Zaloguj się w Steam.").into());
            }

            let engine = create_interface(c"CLIENTENGINE_INTERFACE_VERSION005".as_ptr(), std::ptr::null_mut());
            if engine.is_null() {
                return Err(unsupported("CLIENTENGINE_INTERFACE_VERSION005"));
            }
            let get_rcm: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void =
                std::mem::transmute(vfn(engine, ENGINE_GET_REMOTE_CLIENT_MANAGER));
            let rcm = get_rcm(engine, pipe);
            if rcm.is_null() {
                return Err(tr("Steam did not provide its Remote Play manager (are you signed in?)", "Steam nie udostępnił menedżera Remote Play (czy jesteś zalogowany?)").into());
            }

            let base = module as usize;
            let image = module_image(base).ok_or_else(|| unsupported("PE header"))?;
            let vtable = *(rcm as *const usize);
            if vtable < base || vtable >= base + image.len() {
                return Err(unsupported("IClientEngine layout"));
            }
            let names = scan_vtable_names(image, base, vtable - base);
            log(&format!("rpt: resolved {} IClientRemoteClientManager methods by name", names.len()));
            let mut remote_play: Vec<_> = names.iter().filter(|(n, _)| n.contains("RemotePlay")).map(|(n, i)| format!("{i}:{n}")).collect();
            remote_play.sort();
            log(&format!("rpt: Remote Play methods in this Steam build: {}", remote_play.join(" ")));
            if let Some(missing) = REQUIRED_METHODS.iter().find(|m| !names.contains_key(**m)) {
                return Err(unsupported(missing));
            }

            let host_steam = match crate::steam::active_user() {
                0 => 0,
                account => STEAM_ID_BASE + account as u64,
            };

            Ok(Self {
                rcm,
                names,
                pipe,
                bget,
                bfree,
                appid,
                desktop_stream,
                host_steam,
                guests: Vec::new(),
                max_invited_guest: 0,
                want: inputs,
                guest_connected: false,
                overlay_refreshed: false,
            })
        }
    }

    fn method(&self, name: &str) -> Option<*const c_void> {
        self.names.get(name).map(|&i| unsafe { vfn(self.rcm, i) })
    }

    fn call_void(&self, name: &str) {
        if let Some(f) = self.method(name) {
            let f: unsafe extern "C" fn(*mut c_void) = unsafe { std::mem::transmute(f) };
            unsafe { f(self.rcm) }
        }
    }

    fn call_void_u32(&self, name: &str, arg: u32) {
        if let Some(f) = self.method(name) {
            let f: unsafe extern "C" fn(*mut c_void, u32) = unsafe { std::mem::transmute(f) };
            unsafe { f(self.rcm, arg) }
        }
    }

    fn call_void_bool(&self, name: &str, arg: bool) {
        if let Some(f) = self.method(name) {
            let f: unsafe extern "C" fn(*mut c_void, u8) = unsafe { std::mem::transmute(f) };
            unsafe { f(self.rcm, u8::from(arg)) }
        }
    }

    fn call_u32(&self, name: &str) -> u32 {
        self.method(name).map_or(0, |f| {
            let f: unsafe extern "C" fn(*mut c_void) -> u32 = unsafe { std::mem::transmute(f) };
            unsafe { f(self.rcm) }
        })
    }

    fn call_u32_u32(&self, name: &str, arg: u32) -> u32 {
        self.method(name).map_or(0, |f| {
            let f: unsafe extern "C" fn(*mut c_void, u32) -> u32 = unsafe { std::mem::transmute(f) };
            unsafe { f(self.rcm, arg) }
        })
    }

    /// Id of the group we host for our AppID (0 if none). Current client builds key groups by
    /// AppID; older ones took no argument and ignore the extra register.
    fn group(&self) -> u32 {
        self.call_u32_u32("GetLocalRemotePlayTogetherGroupID", self.appid)
    }

    fn show_overlay(&self) {
        self.call_void_u32("ShowRemotePlayTogetherUI", self.appid);
    }

    /// Drain the callback pipe, copying each payload out before Steam frees it.
    fn poll(&mut self) -> Vec<(i32, Vec<u8>)> {
        let mut out = Vec::new();
        let mut msg = CallbackMsg { user: 0, id: 0, param: std::ptr::null(), size: 0 };
        unsafe {
            while (self.bget)(self.pipe, &mut msg) != 0 {
                let len = (msg.size.max(0) as usize).min(MAX_CALLBACK_PAYLOAD);
                let payload = if msg.param.is_null() || len == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(msg.param, len).to_vec()
                };
                out.push((msg.id, payload));
                (self.bfree)(self.pipe);
            }
        }
        out
    }

    fn pump(&mut self, duration: Duration) {
        let end = Instant::now() + duration;
        while Instant::now() < end {
            let callbacks = self.poll();
            if callbacks.is_empty() {
                std::thread::sleep(Duration::from_millis(25));
            }
            for (id, payload) in callbacks {
                self.handle_callback(id, &payload);
            }
        }
    }

    fn handle_callback(&mut self, id: i32, payload: &[u8]) {
        if !(CB_REMOTE_PLAY_FIRST..CB_REMOTE_PLAY_LAST).contains(&id) || id == CB_INVITE_RESULT {
            return;
        }
        if !CB_GROUP_REFRESH.contains(&id) {
            let hex: String = payload.iter().take(64).map(|b| format!("{b:02x}")).collect();
            log(&format!("rpt: callback {id} ({} bytes): {hex}", payload.len()));
        }
        if id == CB_STREAM_ACTIVE && !self.guest_connected {
            self.guest_connected = true;
            emit(HostEvent::GuestConnected);
        }
        self.discover_guests(payload);
        if CB_MEMBERSHIP.contains(&id) {
            let refresh = !self.overlay_refreshed;
            self.publish_group(refresh);
        }
    }

    fn discover_guests(&mut self, payload: &[u8]) {
        let mut changed = false;
        for found in scan_guests(payload, self.group(), self.host_steam) {
            match self.guests.iter_mut().find(|g| g.steam == found.steam) {
                Some(g) if g.guest_id != found.guest_id => {
                    g.guest_id = found.guest_id;
                    changed = true;
                }
                Some(_) => {}
                None => {
                    let slot = self.guests.len() as u32 + 1;
                    emit(HostEvent::Player { slot, steam: found.steam, name: format!("{} {slot}", tr("Player", "Gracz")) });
                    self.guests.push(found);
                    changed = true;
                }
            }
        }
        if changed {
            self.apply_inputs();
        }
    }

    /// Publish group membership so a connected guest's stream is registered as a player.
    fn publish_group(&mut self, refresh_overlay: bool) {
        self.call_void("UpdateRemotePlayTogetherGroups");
        let group = self.group();
        if group != 0 {
            self.call_void_u32("UpdateRemotePlayTogetherGroup", group);
        }
        if refresh_overlay {
            self.show_overlay();
            self.overlay_refreshed = true;
        }
    }

    fn set_inputs_for(&self, steam: u64, guest_id: u32, group: u32) {
        let player = RemotePlayPlayer { steam_id: steam, guest_id, group_id: group, ..Default::default() };
        for (name, on) in [
            ("SetPerUserKeyboardInputEnabled", self.want.keyboard),
            ("SetPerUserMouseInputEnabled", self.want.mouse),
            ("SetPerUserControllerInputEnabled", self.want.controller),
        ] {
            if let Some(f) = self.method(name) {
                let f: unsafe extern "C" fn(*mut c_void, *const RemotePlayPlayer, u8) = unsafe { std::mem::transmute(f) };
                unsafe { f(self.rcm, &player, u8::from(on)) }
            }
        }
    }

    /// Same effect as the keyboard/mouse/controller icons in the overlay. Re-applied on a
    /// heartbeat because Steam resets these on its own.
    fn apply_inputs(&self) {
        let group = self.group();
        if group == 0 {
            return;
        }
        for (steam, guest_id) in input_targets(self.max_invited_guest, &self.guests) {
            self.set_inputs_for(steam, guest_id, group);
        }
    }

    fn create_group(&mut self) -> Result<u32, String> {
        let existing = self.group();
        if existing != 0 {
            return Ok(existing);
        }
        let accepted = self.call_u32_u32("BCreateRemotePlayGroup", self.appid) & 0xff != 0;
        log(&format!("rpt: BCreateRemotePlayGroup({}) = {accepted}", self.appid));
        // The id is allocated asynchronously
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            self.pump(Duration::from_millis(150));
            let group = self.group();
            if group != 0 {
                return Ok(group);
            }
        }
        Err(tr("Steam did not create a Remote Play Together group", "Steam nie utworzył grupy Remote Play Together").into())
    }

    /// Create one guest slot in the live group and wait for its join link.
    fn add_guest_link(&mut self) -> Option<String> {
        let group = self.group();
        if group == 0 {
            emit(HostEvent::Error(tr("There is no active Remote Play Together group", "Brak aktywnej grupy Remote Play Together").into()));
            return None;
        }
        let guest_id = self.call_u32("GetAvailableRemotePlayTogetherGuestID").max(1);
        self.max_invited_guest = self.max_invited_guest.max(guest_id);

        let player = RemotePlayPlayer { steam_id: 0, guest_id, group_id: group, ..Default::default() };
        let created = self.method("BCreateRemotePlayInviteAndSession").is_some_and(|f| {
            let f: unsafe extern "C" fn(*mut c_void, *const RemotePlayPlayer, u32) -> u8 = unsafe { std::mem::transmute(f) };
            unsafe { f(self.rcm, &player, self.appid) != 0 }
        });
        log(&format!("rpt: BCreateRemotePlayInviteAndSession={created} group={group} guest={guest_id} app={}", self.appid));

        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            let callbacks = self.poll();
            if callbacks.is_empty() {
                std::thread::sleep(Duration::from_millis(30));
            }
            let mut url = None;
            for (id, payload) in callbacks {
                if id == CB_INVITE_RESULT {
                    url = url.or_else(|| extract_url(&payload));
                } else {
                    self.handle_callback(id, &payload);
                }
            }
            if let Some(url) = url {
                let url = normalize_url(&url);
                emit(HostEvent::Link(url.clone()));
                return Some(url);
            }
        }
        emit(HostEvent::Error(tr("Steam did not return an invite link", "Steam nie zwrócił linku zaproszenia").into()));
        None
    }

    /// Disable guest input and disband the group so guests stop streaming. Exiting alone would
    /// leave Steam's stream alive.
    fn teardown(&mut self) {
        self.want = GuestInputs { keyboard: false, mouse: false, controller: false };
        self.apply_inputs();
        let group = self.group();
        if group != 0 {
            if self.names.contains_key("DisbandRemotePlayTogetherGroup") {
                self.call_void_u32("DisbandRemotePlayTogetherGroup", group);
            } else {
                log("rpt: DisbandRemotePlayTogetherGroup not found in this Steam build; group left to Steam");
            }
        }
        if self.desktop_stream {
            self.call_void_bool("SetStreamingDesktopToRemotePlayTogetherEnabled", false);
        }
        self.pump(Duration::from_millis(300));
    }

    fn run(&mut self, commands: Receiver<HostCommand>) {
        let mut last_publish = Instant::now();
        loop {
            let callbacks = self.poll();
            let idle = callbacks.is_empty();
            for (id, payload) in callbacks {
                self.handle_callback(id, &payload);
            }

            loop {
                match commands.try_recv() {
                    Ok(HostCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => return,
                    Ok(HostCommand::AddLink) => {
                        self.add_guest_link();
                    }
                    Ok(HostCommand::SetInput(kind, on)) => {
                        match kind {
                            InputKind::Keyboard => self.want.keyboard = on,
                            InputKind::Mouse => self.want.mouse = on,
                            InputKind::Controller => self.want.controller = on,
                        }
                        self.apply_inputs();
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }

            if last_publish.elapsed() > Duration::from_secs(2) {
                self.publish_group(false);
                if self.guest_connected {
                    self.apply_inputs();
                }
                last_publish = Instant::now();
            }
            if idle {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

/// The loaded module as a byte slice (`SizeOfImage` from its PE header).
unsafe fn module_image(base: usize) -> Option<&'static [u8]> {
    unsafe {
        let header = std::slice::from_raw_parts(base as *const u8, 0x1000);
        if header.get(..2)? != b"MZ" {
            return None;
        }
        let pe = read_u32(header, 0x3C)? as usize;
        if header.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let size = read_u32(header, pe + 0x50)? as usize;
        Some(std::slice::from_raw_parts(base as *const u8, size))
    }
}

/// Entry point of `--host-session`. Returns the process exit code.
pub fn run_host(appid: u32, desktop_stream: bool, inputs: GuestInputs) -> i32 {
    log(&format!("rpt: host starting appid={appid} desktop_stream={desktop_stream} inputs={inputs:?}"));
    status(tr("Connecting to the Steam client...", "Łączenie z klientem Steam..."));
    let mut host = match Host::attach(appid, desktop_stream, inputs) {
        Ok(h) => h,
        Err(e) => {
            emit(HostEvent::Error(e));
            return 2;
        }
    };

    // Commands come from the parent over stdin; EOF (parent gone) ends the session.
    let (tx, commands) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Some(cmd) = HostCommand::parse(&line) {
                if tx.send(cmd).is_err() {
                    return;
                }
            }
        }
        let _ = tx.send(HostCommand::Stop);
    });

    if desktop_stream {
        // Game is not tracked by Steam, so there is no hooked game window to capture.
        host.call_void_bool("SetStreamingDesktopToRemotePlayTogetherEnabled", true);
    }

    status(tr("Creating the Remote Play Together group...", "Tworzenie grupy Remote Play Together..."));
    let group = match host.create_group() {
        Ok(g) => g,
        Err(e) => {
            emit(HostEvent::Error(e));
            host.teardown();
            return 5;
        }
    };
    log(&format!("rpt: group {group} live"));

    status(tr("Creating the invite link...", "Tworzenie linku zaproszenia..."));
    host.add_guest_link();
    host.show_overlay();
    emit(HostEvent::Ready);

    host.run(commands);

    status(tr("Ending the session...", "Kończenie sesji..."));
    host.teardown();
    emit(HostEvent::Stopped);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: usize = 0x1_8000_0000;

    fn put(image: &mut [u8], off: usize, bytes: &[u8]) {
        image[off..off + bytes.len()].copy_from_slice(bytes);
    }

    /// `lea reg, [rip+disp]` at `at` pointing to `target`.
    fn lea(image: &mut [u8], at: usize, prefix: [u8; 3], target: usize) {
        let disp = (target as i64 - (at as i64 + 7)) as i32;
        put(image, at, &prefix);
        put(image, at + 3, &disp.to_le_bytes());
    }

    fn fake_image() -> Vec<u8> {
        let mut image = vec![0u8; 0x1000];
        // vtable at 0x100: two stubs, then a pointer outside the module
        put(&mut image, 0x100, &((BASE + 0x200) as u64).to_le_bytes());
        put(&mut image, 0x108, &((BASE + 0x300) as u64).to_le_bytes());
        put(&mut image, 0x110, &0x7FFF_0000_0000u64.to_le_bytes());
        put(&mut image, 0x118, &((BASE + 0x200) as u64).to_le_bytes());
        // stub 0: some prologue, then lea rax,[rip+..] -> name
        put(&mut image, 0x200, &[0x48, 0x89, 0x5C, 0x24, 0x08, 0x57]);
        lea(&mut image, 0x206, [0x48, 0x8D, 0x05], 0x800);
        put(&mut image, 0x800, b"BCreateRemotePlayGroup\0");
        // stub 1: first lea targets a string with a space (rejected), second is the name
        lea(&mut image, 0x300, [0x48, 0x8D, 0x15], 0x900);
        put(&mut image, 0x900, b"not a name\0");
        lea(&mut image, 0x310, [0x4C, 0x8D, 0x0D], 0x840);
        put(&mut image, 0x840, b"GetLocalRemotePlayTogetherGroupID\0");
        image
    }

    #[test]
    fn stub_name_follows_rip_relative_lea() {
        let image = fake_image();
        assert_eq!(stub_name(&image, 0x200).as_deref(), Some("BCreateRemotePlayGroup"));
        assert_eq!(stub_name(&image, 0x300).as_deref(), Some("GetLocalRemotePlayTogetherGroupID"));
    }

    #[test]
    fn stub_name_none_without_string_and_at_image_end() {
        let image = vec![0xCCu8; 0x400];
        assert_eq!(stub_name(&image, 0x10), None);
        assert_eq!(stub_name(&image, 0x3FE), None);
    }

    #[test]
    fn stub_name_rejects_out_of_image_target() {
        let mut image = vec![0u8; 0x400];
        put(&mut image, 0x10, &[0x48, 0x8D, 0x05]);
        put(&mut image, 0x13, &0x7000_0000i32.to_le_bytes());
        assert_eq!(stub_name(&image, 0x10), None);
    }

    #[test]
    fn vtable_scan_maps_names_and_stops_at_foreign_pointer() {
        let image = fake_image();
        let map = scan_vtable_names(&image, BASE, 0x100);
        assert_eq!(map.len(), 2);
        assert_eq!(map["BCreateRemotePlayGroup"], 0);
        assert_eq!(map["GetLocalRemotePlayTogetherGroupID"], 1);
    }

    #[test]
    fn url_at_known_offset() {
        let mut payload = vec![0u8; 200];
        put(&mut payload, 56, b"s.team/remoteplay/connect/1234\0");
        assert_eq!(extract_url(&payload).as_deref(), Some("s.team/remoteplay/connect/1234"));
    }

    #[test]
    fn url_found_when_struct_layout_moves() {
        let mut payload = vec![0u8; 200];
        put(&mut payload, 56, &[1, 2, 3, 0]);
        put(&mut payload, 72, b"https://s.team/remoteplay/connect/9\0");
        assert_eq!(extract_url(&payload).as_deref(), Some("https://s.team/remoteplay/connect/9"));
        assert_eq!(extract_url(&[0u8; 200]), None);
        assert_eq!(extract_url(&[]), None);
    }

    #[test]
    fn url_normalization() {
        assert_eq!(normalize_url("s.team/p/abcd"), "https://s.team/p/abcd");
        assert_eq!(normalize_url("//s.team/p/abcd"), "https://s.team/p/abcd");
        assert_eq!(normalize_url(" https://s.team/p/abcd "), "https://s.team/p/abcd");
        assert_eq!(normalize_url("steam://remoteplay/connect/1"), "steam://remoteplay/connect/1");
    }

    fn player(steam: u64, guest: u32, group: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&steam.to_le_bytes());
        b.extend_from_slice(&guest.to_le_bytes());
        b.extend_from_slice(&group.to_le_bytes());
        b
    }

    #[test]
    fn guests_found_only_in_live_group() {
        let host = STEAM_ID_BASE + 111;
        let friend = STEAM_ID_BASE + 222;
        let mut payload = vec![0u8; 8];
        payload.extend(player(host, 1, 7)); // host itself
        payload.extend(player(friend, 2, 7)); // real guest
        payload.extend(player(STEAM_ID_BASE + 333, 3, 9)); // other group
        payload.extend(player(0, 1, 7)); // unclaimed link slot
        payload.extend(player(friend, 2, 7)); // duplicate

        assert_eq!(scan_guests(&payload, 7, host), vec![Guest { steam: friend, guest_id: 2 }]);
    }

    #[test]
    fn invite_created_callback_is_not_a_guest() {
        // Real 1100011 payload head from Steam when a link slot is created (guest 1, group 3)
        let hex = "01000000000000000000000001000000030000000000000000000000000000000000000000000000d2c4040000000000030000000101010000187d6cf3010000";
        let payload: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        assert!(scan_guests(&payload, 3, STEAM_ID_BASE + 111).is_empty());
    }

    #[test]
    fn guests_none_without_group_or_on_short_payload() {
        let payload = player(STEAM_ID_BASE + 222, 1, 7);
        assert!(scan_guests(&payload, 0, 0).is_empty());
        assert!(scan_guests(&payload[..12], 7, 0).is_empty());
        assert!(scan_guests(&[], 7, 0).is_empty());
    }

    #[test]
    fn claimed_link_slot_is_addressed_only_by_its_account() {
        let friend = STEAM_ID_BASE + 222;
        assert_eq!(input_targets(2, &[]), vec![(0, 1), (0, 2)]);
        assert_eq!(input_targets(2, &[Guest { steam: friend, guest_id: 1 }]), vec![(0, 2), (friend, 1)]);
        assert_eq!(input_targets(0, &[]), vec![]);
    }

    #[test]
    fn remote_play_player_layout() {
        assert_eq!(std::mem::size_of::<RemotePlayPlayer>(), 40);
    }
}

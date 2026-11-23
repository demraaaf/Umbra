//! Where games are installed on this PC, and what they are called.
//!
//! Game stores keep a list of what they installed: Steam in its library folders and app
//! manifests, Epic in its manifests, GOG, Ubisoft and EA in the registry. Many games also sit
//! in a plain `Games` folder or under a publisher's folder (`Riot Games`, `XboxGames`). A
//! program run from one of these places is a game, windowed or not, unless it is one of the
//! helpers that ship next to games (crash reporters, anti-cheat setups, launchers).
//!
//! Names come from the store when it knows the game ("Broken Arrow" for `BrokenArrow.exe`),
//! else from the program's own version info ("World of Warships"), else from its folder or
//! its file name. Only files and the registry are read; no game process is touched.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A folder games are installed in.
#[derive(Clone, Debug)]
struct Root {
    /// Lowercase, backslashes, ending in a backslash.
    prefix: String,
    /// The game's name, when the folder is one game's.
    name: Option<String>,
    /// For example `Steam`, `Epic Games`, `a Games folder`.
    store: &'static str,
    /// A tool the store installed (Wallpaper Engine, a redistributable), not a game.
    tool: bool,
}

/// Store entries that are tools, not games: their programs never count as games.
fn is_tool(name: &str) -> bool {
    const TOOLS: &[&str] = &[
        "redistributable",
        "steamworks",
        "wallpaper engine",
        "soundpad",
        "crosshair x",
        "lossless scaling",
        "steamvr",
        "quixel",
        "unreal engine",
        "obs studio",
        "blender",
        "proton",
        "steam linux runtime",
        "sdk",
        "dedicated server",
        "editor",
        "fpsvr",
        "voicemod",
        "streamlabs",
        "spacedesk",
    ];
    let n = name.to_lowercase();
    TOOLS.iter().any(|t| n.contains(t))
}

#[derive(Default)]
struct Library {
    roots: Vec<Root>,
}

/// Why a program counts as a game, or where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub store: &'static str,
    /// The store's name for it, if it has one.
    pub name: Option<String>,
}

const RESCAN: Duration = Duration::from_secs(300);

static LIBRARY: Mutex<Option<(Instant, Arc<Library>)>> = Mutex::new(None);
static NAMES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn library() -> Arc<Library> {
    let mut slot = LIBRARY.lock().unwrap();
    if let Some((at, lib)) = slot.as_ref() {
        if at.elapsed() < RESCAN {
            return lib.clone();
        }
    }
    let lib = Arc::new(scan());
    *slot = Some((Instant::now(), lib.clone()));
    lib
}

fn normalize(path: &str) -> String {
    let mut p = path.replace('/', "\\").to_lowercase();
    if !p.ends_with('\\') {
        p.push('\\');
    }
    p
}

/// Folder names that are a store's or a publisher's, not a game's.
const PUBLISHER_FOLDERS: &[&str] = &[
    "games",
    "game",
    "riot games",
    "rockstar games",
    "epic",
    "epic games",
    "ubisoft",
    "ubisoft game launcher",
    "ea games",
    "ea",
    "electronic arts",
    "steam",
    "steamlibrary",
    "steamapps",
    "common",
    "gog",
    "gog games",
    "gog galaxy",
    "xboxgames",
    "wargaming.net",
    "lesta",
    "battle.net",
    "blizzard",
    "blizzard entertainment",
    "program files",
    "program files (x86)",
    "content",
];

/// Folders whose programs are games.
const GAME_FOLDERS: &[(&str, &'static str)] = &[
    ("\\steamapps\\common\\", "Steam"),
    ("\\epic games\\", "Epic Games"),
    ("\\riot games\\", "Riot Games"),
    ("\\rockstar games\\", "Rockstar Games"),
    ("\\ea games\\", "EA"),
    ("\\gog games\\", "GOG"),
    ("\\gog galaxy\\games\\", "GOG"),
    ("\\ubisoft game launcher\\games\\", "Ubisoft Connect"),
    ("\\xboxgames\\", "Xbox"),
    ("\\games\\", "a Games folder"),
];

fn scan() -> Library {
    let mut roots = Vec::new();
    steam(&mut roots);
    epic(&mut roots);
    gog(&mut roots);
    ubisoft(&mut roots);
    ea(&mut roots);
    // The longest folder wins, so a game's own folder beats the library it is in.
    roots.sort_by_key(|r| std::cmp::Reverse(r.prefix.len()));
    roots.dedup_by(|a, b| a.prefix == b.prefix);
    log::info!("game library: {} game folders known", roots.len());
    Library { roots }
}

/// Steam's library folders, the one Steam is installed in first.
fn steam_libraries() -> Vec<PathBuf> {
    let Some(base) = reg::string(reg::Root::CurrentUser, r"Software\Valve\Steam", "SteamPath")
    else {
        return Vec::new();
    };
    let base = PathBuf::from(base.replace('/', "\\"));
    let mut libraries = vec![base.clone()];
    if let Ok(text) = std::fs::read_to_string(base.join(r"steamapps\libraryfolders.vdf")) {
        libraries.extend(vdf_values(&text, "path").into_iter().map(PathBuf::from));
    }
    libraries.dedup();
    libraries
}

/// The folder a Steam game is installed in, by its app id (730 is Counter-Strike 2).
pub fn steam_app_dir(app_id: u32) -> Option<PathBuf> {
    steam_libraries().into_iter().find_map(|library| {
        let apps = library.join("steamapps");
        let text = std::fs::read_to_string(apps.join(format!("appmanifest_{app_id}.acf"))).ok()?;
        let dir = vdf_values(&text, "installdir").into_iter().next()?;
        Some(apps.join("common").join(dir)).filter(|p| p.is_dir())
    })
}

fn steam(roots: &mut Vec<Root>) {
    for library in steam_libraries() {
        let apps = library.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&apps) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let (Some(title), Some(dir)) = (
                vdf_values(&text, "name").into_iter().next(),
                vdf_values(&text, "installdir").into_iter().next(),
            ) else {
                continue;
            };
            roots.push(Root {
                prefix: normalize(&apps.join("common").join(dir).to_string_lossy()),
                tool: is_tool(&title),
                name: Some(title),
                store: "Steam",
            });
        }
    }
}

/// The values of `"key" "value"` pairs in Valve's text format, unescaped.
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix(&needle)?.trim();
            let value = rest.strip_prefix('"')?.strip_suffix('"')?;
            Some(value.replace("\\\\", "\\"))
        })
        .collect()
}

fn epic(roots: &mut Vec<Root>) {
    let folder = Path::new(r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .path()
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("item"))
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let (Some(dir), Some(name)) = (
            json["InstallLocation"].as_str(),
            json["DisplayName"].as_str(),
        ) {
            roots.push(Root {
                prefix: normalize(dir),
                name: Some(name.to_string()),
                store: "Epic Games",
                tool: is_tool(name),
            });
        }
    }
}

fn gog(roots: &mut Vec<Root>) {
    let key = r"SOFTWARE\WOW6432Node\GOG.com\Games";
    for id in reg::subkeys(reg::Root::LocalMachine, key) {
        let sub = format!(r"{key}\{id}");
        if let Some(path) = reg::string(reg::Root::LocalMachine, &sub, "path") {
            roots.push(Root {
                prefix: normalize(&path),
                name: reg::string(reg::Root::LocalMachine, &sub, "gameName"),
                store: "GOG",
                tool: false,
            });
        }
    }
}

fn ubisoft(roots: &mut Vec<Root>) {
    let key = r"SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs";
    for id in reg::subkeys(reg::Root::LocalMachine, key) {
        if let Some(dir) = reg::string(
            reg::Root::LocalMachine,
            &format!(r"{key}\{id}"),
            "InstallDir",
        ) {
            roots.push(Root {
                prefix: normalize(&dir),
                name: None,
                store: "Ubisoft Connect",
                tool: false,
            });
        }
    }
}

fn ea(roots: &mut Vec<Root>) {
    let key = r"SOFTWARE\WOW6432Node\EA Games";
    for game in reg::subkeys(reg::Root::LocalMachine, key) {
        if let Some(dir) = reg::string(
            reg::Root::LocalMachine,
            &format!(r"{key}\{game}"),
            "Install Dir",
        ) {
            roots.push(Root {
                prefix: normalize(&dir),
                tool: is_tool(&game),
                name: Some(game),
                store: "EA",
            });
        }
    }
}

/// Where the program at `path` was installed from, if it is in a place games are.
pub fn installed(path: &str) -> Option<Installed> {
    let p = normalize(path);
    if let Some(root) = library().roots.iter().find(|r| p.starts_with(&r.prefix)) {
        if root.tool {
            return None;
        }
        return Some(Installed {
            store: root.store,
            name: root.name.clone(),
        });
    }
    GAME_FOLDERS
        .iter()
        .find(|(folder, _)| p.contains(folder))
        .map(|(_, store)| Installed { store, name: None })
}

/// Programs that ship next to games and are not the game.
pub fn is_helper(exe: &str) -> bool {
    const PARTS: &[&str] = &[
        "crash",
        "report",
        "setup",
        "install",
        "unins",
        "redist",
        "launcher",
        "easyanticheat",
        "eac_",
        "eaclauncher",
        "battleye",
        "beservice",
        "helper",
        "updater",
        "patcher",
        "prereq",
        "dxsetup",
        "elevate",
        "cefprocess",
        "webview",
        "overlay",
        "service",
        "bootstrap",
        "repair",
        "touchup",
        "cleanup",
        "benchmark",
        "editor",
        "modmanager",
        "server",
    ];
    let e = exe.to_lowercase();
    PARTS.iter().any(|p| e.contains(p))
}

/// A readable name for the game `exe` at `path`, remembered for `game_name_of`.
pub fn display_name(exe: &str, path: Option<&str>) -> String {
    let key = exe.to_lowercase();
    if let Some(name) = NAMES.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        return name.clone();
    }
    let name = path
        .and_then(installed)
        .and_then(|i| i.name)
        .map(|n| clean(&n))
        .filter(|n| !n.is_empty())
        .or_else(|| path.and_then(version_name))
        .or_else(|| path.and_then(folder_name))
        .unwrap_or_else(|| name_from_file(exe));
    NAMES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key, name.clone());
    name
}

/// The name a game was given when it was detected, if it was.
pub fn known_name(exe: &str) -> Option<String> {
    NAMES
        .lock()
        .unwrap()
        .as_ref()?
        .get(&exe.to_lowercase())
        .cloned()
}

/// Trademark signs and spare spaces out.
fn clean(name: &str) -> String {
    let stripped: String = name
        .replace("(TM)", "")
        .replace("(R)", "")
        .chars()
        .filter(|c| !matches!(c, '™' | '®' | '©'))
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Names that say nothing about which game it is.
fn generic(name: &str) -> bool {
    let n = name.to_lowercase();
    n.len() < 2
        || n.ends_with(".exe")
        || n.contains('\\')
        || [
            "unreal",
            "engine",
            "unity",
            "launcher",
            "bootstrap",
            "microsoft",
            "windows",
            "game client",
            "sentry",
            "application",
        ]
        .iter()
        .any(|g| n.contains(g))
        || n == "game"
}

/// The program's own name from its version info: the description or the product name,
/// preferring one written with spaces ("World of Warships" over "WorldOfWarships").
fn version_name(path: &str) -> Option<String> {
    let (description, product) = version::strings(path);
    let candidates: Vec<String> = [description, product]
        .into_iter()
        .flatten()
        .map(|s| clean(&s))
        .filter(|s| !generic(s))
        .collect();
    candidates
        .iter()
        .find(|s| s.contains(' '))
        .or_else(|| candidates.first())
        .cloned()
}

/// The first folder under a Games folder or a store's that is not the store's own.
fn folder_name(path: &str) -> Option<String> {
    let original = path.replace('/', "\\");
    let segments: Vec<&str> = original.split('\\').collect();
    let lower: Vec<String> = segments.iter().map(|s| s.to_lowercase()).collect();
    const MARKERS: &[&str] = &[
        "games",
        "epic games",
        "riot games",
        "rockstar games",
        "ea games",
        "gog games",
        "xboxgames",
    ];
    let start = lower.iter().enumerate().position(|(i, s)| {
        MARKERS.contains(&s.as_str()) || (s == "common" && i > 0 && lower[i - 1] == "steamapps")
    })?;
    // The last segment is the program itself.
    segments[start + 1..segments.len().saturating_sub(1).max(start + 1)]
        .iter()
        .find(|f| !PUBLISHER_FOLDERS.contains(&f.to_lowercase().as_str()))
        .map(|f| tidy(f))
}

/// `World_of_Warships` → `World of Warships`, `WorldOfWarships64` → `World Of Warships`.
fn tidy(raw: &str) -> String {
    let mut s = raw.replace(['_', '-'], " ");
    for suffix in [
        " Win64 Shipping",
        " Shipping",
        " Win64",
        " x64",
        " dx12",
        " dx11",
        " BE",
        " EAC",
    ] {
        if s.to_lowercase().ends_with(&suffix.to_lowercase()) {
            s.truncate(s.len() - suffix.len());
        }
    }
    let s = s.trim_end_matches("64").trim_end_matches("32").trim();
    // Split CamelCase where a lowercase letter meets an uppercase one.
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase();
        out.push(c);
    }
    clean(&out)
}

fn name_from_file(exe: &str) -> String {
    let stem = exe
        .strip_suffix(".exe")
        .or_else(|| exe.strip_suffix(".EXE"))
        .unwrap_or(exe);
    let tidy = tidy(stem);
    if tidy.is_empty() {
        stem.to_string()
    } else {
        tidy
    }
}

mod version {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    use windows::core::PCWSTR;

    /// The file's description and product name from its version resource.
    pub fn strings(path: &str) -> (Option<String>, Option<String>) {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
            if size == 0 {
                return (None, None);
            }
            let mut data = vec![0u8; size as usize];
            if GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr().cast())
                .is_err()
            {
                return (None, None);
            }
            // The first language and code page the file lists, else US English, Unicode.
            let mut ptr = std::ptr::null_mut();
            let mut len = 0u32;
            let translation: Vec<u16> = "\\VarFileInfo\\Translation"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let lang = if VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(translation.as_ptr()),
                &mut ptr,
                &mut len,
            )
            .as_bool()
                && len >= 4
            {
                let pair = std::slice::from_raw_parts(ptr as *const u16, 2);
                format!("{:04x}{:04x}", pair[0], pair[1])
            } else {
                "040904b0".to_string()
            };
            let read = |field: &str| -> Option<String> {
                let query: Vec<u16> = format!("\\StringFileInfo\\{lang}\\{field}")
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                let mut ptr = std::ptr::null_mut();
                let mut len = 0u32;
                if !VerQueryValueW(
                    data.as_ptr().cast(),
                    PCWSTR(query.as_ptr()),
                    &mut ptr,
                    &mut len,
                )
                .as_bool()
                    || len == 0
                {
                    return None;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(
                    ptr as *const u16,
                    len as usize,
                ));
                let text = text.trim_end_matches('\0').trim().to_string();
                (!text.is_empty()).then_some(text)
            };
            (read("FileDescription"), read("ProductName"))
        }
    }
}

mod reg {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey,
        RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    };
    use windows::core::{PCWSTR, PWSTR};

    pub enum Root {
        CurrentUser,
        LocalMachine,
    }

    fn hkey(root: Root) -> HKEY {
        match root {
            Root::CurrentUser => HKEY_CURRENT_USER,
            Root::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn string(root: Root, key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buffer = vec![0u16; 1024];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                hkey(root),
                PCWSTR(key.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1);
        let text = String::from_utf16_lossy(&buffer[..len.min(buffer.len())]);
        let text = text.trim_end_matches('\0').to_string();
        (!text.is_empty()).then_some(text)
    }

    pub fn subkeys(root: Root, key: &str) -> Vec<String> {
        let key = wide(key);
        let mut handle = HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                hkey(root),
                PCWSTR(key.as_ptr()),
                None,
                KEY_READ,
                &mut handle,
            )
        } != ERROR_SUCCESS
        {
            return Vec::new();
        }
        let mut names = Vec::new();
        for index in 0.. {
            let mut buffer = [0u16; 256];
            let mut len = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    handle,
                    index,
                    Some(PWSTR(buffer.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&buffer[..len as usize]));
        }
        unsafe {
            let _ = RegCloseKey(handle);
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_tidied() {
        assert_eq!(tidy("World_of_Warships"), "World of Warships");
        assert_eq!(tidy("WorldOfWarships64"), "World Of Warships");
        assert_eq!(tidy("VALORANT-Win64-Shipping"), "VALORANT");
        assert_eq!(clean("Battlefield™ V"), "Battlefield V");
        assert_eq!(name_from_file("bf1.exe"), "bf1");
    }

    #[test]
    fn folders_give_names_past_the_publisher() {
        assert_eq!(
            folder_name(r"D:\Games\World_of_Warships\bin\13187581\bin64\WorldOfWarships64.exe")
                .as_deref(),
            Some("World of Warships")
        );
        assert_eq!(
            folder_name(r"D:\Games\Riot Games\VALORANT\live\VALORANT.exe").as_deref(),
            Some("VALORANT")
        );
        assert_eq!(folder_name(r"C:\Tools\thing.exe"), None);
    }

    #[test]
    fn helpers_are_not_games() {
        for exe in [
            "UnityCrashHandler64.exe",
            "crashpad_handler.exe",
            "EACLauncher.exe",
            "EasyAntiCheat_EOS_Setup.exe",
            "BEService.exe",
            "unins000.exe",
        ] {
            assert!(is_helper(exe), "{exe}");
        }
        for exe in [
            "BrokenArrow.exe",
            "WorldOfWarships64.exe",
            "bfv.exe",
            "Among Us.exe",
        ] {
            assert!(!is_helper(exe), "{exe}");
        }
    }

    #[test]
    fn games_folders_count_without_a_store() {
        let found =
            installed(r"D:\Games\World_of_Warships\bin\1\bin64\WorldOfWarships64.exe").unwrap();
        assert!(!found.store.is_empty());
        assert!(installed(r"C:\Program Files\Mozilla Firefox\firefox.exe").is_none());
    }

    #[test]
    fn vdf_values_are_read() {
        let text =
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(
            vdf_values(text, "path"),
            vec![r"D:\SteamLibrary".to_string()]
        );
    }
}

#[cfg(test)]
mod this_pc {
    use super::*;

    /// Prints what this PC's stores and games folders give for a few programs.
    #[test]
    #[ignore]
    fn prints_installed_games() {
        let lib = library();
        println!("{} game folders", lib.roots.len());
        for r in lib.roots.iter().filter(|r| r.name.is_some()).take(40) {
            println!("  {} | {:?} | {}", r.store, r.name, r.prefix);
        }
        for path in [
            r"D:\SteamLibrary\steamapps\common\broken_arrow\BrokenArrow.exe",
            r"D:\SteamLibrary\steamapps\common\Among Us\Among Us.exe",
            r"D:\Games\World_of_Warships\bin\13187581\bin64\WorldOfWarships64.exe",
            r"C:\Program Files\Mozilla Firefox\firefox.exe",
        ] {
            let exe = path.rsplit('\\').next().unwrap();
            println!(
                "{path}\n  installed: {:?}\n  name: {}",
                installed(path),
                display_name(exe, Some(path))
            );
        }
    }
}

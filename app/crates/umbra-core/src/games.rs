//! Finding out whether a game is running. Umbra never touches the game: it only looks at
//! which window is in front, what kind of window it is, where its program is installed and
//! which programs exist, which is what makes it safe with anti-cheat.
//!
//! The window in front counts as a game when its program is on the user's list, or when it
//! is installed where games are (a store's library or a Games folder, see `gamedb`), or when
//! its window is a game engine's, or when it fills its screen; never when it is one of the
//! programs known not to be games, a helper that ships with games, or on the never list.

use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible,
};
use windows::core::PWSTR;

use crate::process::list_processes;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameInfo {
    pub pid: u32,
    /// Executable file name, for example `eldenring.exe`.
    pub exe: String,
    /// Full path of the program, when Windows tells it.
    pub path: Option<String>,
    pub title: String,
    /// What it is called: the store's name, the program's own, or its file name tidied up.
    pub name: String,
    /// Why it counts as a game, for the panel and the log.
    pub reason: String,
    /// The monitor (raw HMONITOR) the game's window filled when it was last in front. While
    /// the user is elsewhere, a fullscreen game is minimized and a stretched one is back at
    /// the desktop's resolution, so its window no longer says how it is played.
    pub screen: Option<isize>,
}

/// Window classes game engines give their windows. Asking a window for its class does not
/// touch its program.
const ENGINE_CLASSES: &[&str] = &[
    "UnrealWindow",
    "UnityWndClass",
    "Valve001",
    "SDL_app",
    "CryENGINE",
    "GLFW30",
    "LWJGL",
    "Godot_Engine",
    "RiotWindowClass",
    "Respawn001",
    "YYGameMakerYY",
    "OgreD3D11Wnd",
    "OgreGLWindow",
    "Frostbite",
    "Bevy",
];

/// Programs that often fill the screen or live next to games without being games.
const NOT_GAMES: &[&str] = &[
    // Store apps and tools installed through Steam or next to games
    "steam.exe",
    "epicgameslauncher.exe",
    "eadesktop.exe",
    "ealauncher.exe",
    "battle.net.exe",
    "galaxyclient.exe",
    "upc.exe",
    "ubisoftconnect.exe",
    "riotclientservices.exe",
    "riotclientux.exe",
    "riotclientuxrender.exe",
    "lgc.exe",
    "wgc.exe",
    "rockstarlauncher.exe",
    "playnite.desktopapp.exe",
    "playnite.fullscreenapp.exe",
    "soundpad.exe",
    "wallpaper32.exe",
    "wallpaper64.exe",
    "ui32.exe",
    "crosshairx.exe",
    "losslessscaling.exe",
    "vrmonitor.exe",
    "vrserver.exe",
    "vrcompositor.exe",
    "overwolf.exe",
    "medal.exe",
    // An Electron app that was never renamed (a tool or launcher), and a World of Warships
    // stats window that fills the screen.
    "electron.exe",
    "wows monitor desktop.exe",
    "nvidia app.exe",
    "nvcplui.exe",
    "nvidia share.exe",
    "gamebar.exe",
    "xboxpcapp.exe",
    "unrealeditor.exe",
    "ue4editor.exe",
    "unity.exe",
    "unity hub.exe",
    "bridge.exe",
    // Video and pictures
    "mpc-be64.exe",
    "potplayer64.exe",
    "kmplayer64.exe",
    "smplayer.exe",
    "kodi.exe",
    "plex.exe",
    "plex htpc.exe",
    "stremio.exe",
    "netflix.exe",
    "video.ui.exe",
    "microsoft.media.player.exe",
    "photos.exe",
    "microsoft.photos.exe",
    "photoshop.exe",
    "lightroom.exe",
    "resolve.exe",
    "premiere pro.exe",
    "afterfx.exe",
    "obs32.exe",
    "streamlabs obs.exe",
    // Office and reading
    "powerpnt.exe",
    "winword.exe",
    "excel.exe",
    "acrobat.exe",
    "acrord32.exe",
    "sumatrapdf.exe",
    // Editors and terminals
    "idea64.exe",
    "rider64.exe",
    "pycharm64.exe",
    "clion64.exe",
    "webstorm64.exe",
    "cursor.exe",
    "sublime_text.exe",
    "notepad++.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "warp.exe",
    "mstsc.exe",
    "vmware.exe",
    "virtualbox.exe",
    "parsecd.exe",
    "explorer.exe",
    "applicationframehost.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "lockapp.exe",
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "vlc.exe",
    "mpc-hc64.exe",
    "mpv.exe",
    "potplayermini64.exe",
    "obs64.exe",
    "discord.exe",
    "slack.exe",
    "teams.exe",
    "zoom.exe",
    "code.exe",
    "devenv.exe",
    "windowsterminal.exe",
    "cmd.exe",
    "powershell.exe",
    "claude.exe",
    "umbra.exe",
    "umbra-cli.exe",
    "steamwebhelper.exe",
    "spotify.exe",
    "notepad.exe",
    "mspaint.exe",
    "blender.exe",
    // Screenshot tools and screen overlays: they cover the screen for a moment over a game
    "snippingtool.exe",
    "screenclippinghost.exe",
    "screensketch.exe",
    "sharex.exe",
    "greenshot.exe",
    "lightshot.exe",
    "flameshot.exe",
    "snagit32.exe",
    "snagiteditor.exe",
    "snagitcapture.exe",
    "picpick.exe",
    "screentogif.exe",
    "magnify.exe",
    "powertoys.colorpickerui.exe",
    "powertoys.powerocr.exe",
    "powertoys.measuretoolui.exe",
];

/// Why a program counts as a game when nothing but its window filling the screen says so.
pub const FILLS_THE_SCREEN: &str = "fills the screen";

fn exe_of(pid: u32) -> Option<String> {
    image_path(pid).and_then(|path| path.rsplit('\\').next().map(str::to_string))
}

/// The full path of the program running as `pid`.
pub fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 520];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn class_of(window: HWND) -> String {
    let mut buffer = [0u16; 256];
    let len =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

/// The window is big enough to be played in, not a splash screen or a dialog.
fn big_enough(window: HWND) -> bool {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect) }.is_ok()
        && rect.right - rect.left >= 640
        && rect.bottom - rect.top >= 360
}

/// Known not to be a game by its name.
pub fn is_not_game(exe: &str) -> bool {
    NOT_GAMES.iter().any(|n| n.eq_ignore_ascii_case(exe))
        || exe.to_lowercase().ends_with(".scr")
        || crate::gamedb::is_helper(exe)
}

fn info(
    pid: u32,
    exe: String,
    path: Option<String>,
    window: Option<HWND>,
    reason: String,
) -> GameInfo {
    let name = crate::gamedb::display_name(&exe, path.as_deref());
    GameInfo {
        pid,
        title: window.map(title_of).unwrap_or_default(),
        screen: window.and_then(|w| filled_monitor(w.0 as isize)),
        name,
        reason,
        path,
        exe,
    }
}

fn title_of(window: HWND) -> String {
    let mut buffer = [0u16; 256];
    let len = unsafe { GetWindowTextW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn covers_monitor(window: HWND) -> bool {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom
    }
}

/// The monitor (raw HMONITOR) that the window `window` (raw HWND) fills, if it fills one: a
/// fullscreen or borderless game.
pub fn filled_monitor(window: isize) -> Option<isize> {
    let window = HWND(window as *mut _);
    if unsafe { IsIconic(window) }.as_bool() || !covers_monitor(window) {
        return None;
    }
    Some(unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) }.0 as isize)
}

/// The game currently running, if any.
///
/// A program the user marked as a game counts whenever it is running. Otherwise the front
/// window counts if it fills its whole monitor and isn't a known non-game program. Programs
/// in `never` never count.
pub fn detect_game(user_games: &[String], never: &[String]) -> Option<GameInfo> {
    let foreground = unsafe { GetForegroundWindow() };
    let allowed = |exe: &str, pid: u32| !crate::apps::listed(never, exe, pid);

    if !user_games.is_empty() {
        let running = list_processes();
        // Prefer the front window's program when it is one of the listed games.
        if !foreground.is_invalid() {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(foreground, Some(&mut pid)) };
            if let Some(exe) = exe_of(pid) {
                if crate::apps::listed(user_games, &exe, pid) && allowed(&exe, pid) {
                    let path = crate::apps::path_of(pid, &exe);
                    return Some(info(
                        pid,
                        exe,
                        path,
                        Some(foreground),
                        "on your games list".into(),
                    ));
                }
            }
        }
        // Otherwise the first listed game that is running at all, taking its top-level process.
        let listed: Vec<_> = running
            .iter()
            .filter(|p| crate::apps::listed(user_games, &p.exe, p.pid) && allowed(&p.exe, p.pid))
            .collect();
        let roots = listed
            .iter()
            .filter(|p| !listed.iter().any(|q| q.pid == p.parent_pid));
        if let Some(root) = roots.into_iter().next() {
            let path = crate::apps::path_of(root.pid, &root.exe);
            return Some(info(
                root.pid,
                root.exe.clone(),
                path,
                None,
                "on your games list".into(),
            ));
        }
    }

    if foreground.is_invalid()
        || !unsafe { IsWindowVisible(foreground) }.as_bool()
        || unsafe { IsIconic(foreground) }.as_bool()
    {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut pid)) };
    if pid == std::process::id() {
        return None;
    }
    let exe = exe_of(pid)?;
    if is_not_game(&exe) || !allowed(&exe, pid) {
        return None;
    }
    let path = image_path(pid);
    let reason = if let Some(found) = path
        .as_deref()
        .and_then(crate::gamedb::installed)
        .filter(|_| big_enough(foreground))
    {
        if found.store == "a Games folder" {
            "installed in a Games folder".to_string()
        } else {
            format!("installed through {}", found.store)
        }
    } else if ENGINE_CLASSES
        .iter()
        .any(|c| class_of(foreground).eq_ignore_ascii_case(c))
        && big_enough(foreground)
    {
        "made with a game engine".to_string()
    } else if covers_monitor(foreground) {
        FILLS_THE_SCREEN.to_string()
    } else {
        return None;
    };
    Some(info(pid, exe, path, Some(foreground), reason))
}

/// A game running now, to pick from a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningGame {
    pub exe: String,
    pub name: String,
}

/// The games running now, in or out of front: the one found, programs on `user_games`, and
/// programs installed where games are. Looks only at which programs run and where they are.
pub fn running_games(user_games: &[String], found: Option<&GameInfo>) -> Vec<RunningGame> {
    let mut out: Vec<RunningGame> = Vec::new();
    let mut add = |exe: &str, name: String| {
        if !out.iter().any(|g| g.exe.eq_ignore_ascii_case(exe)) {
            out.push(RunningGame {
                exe: exe.to_string(),
                name,
            });
        }
    };
    if let Some(game) = found {
        add(&game.exe, game.name.clone());
    }
    let own = std::process::id();
    for process in list_processes() {
        if process.pid == own || is_not_game(&process.exe) {
            continue;
        }
        let path = image_path(process.pid);
        let listed = crate::apps::listed(user_games, &process.exe, process.pid);
        if listed || path.as_deref().and_then(crate::gamedb::installed).is_some() {
            add(
                &process.exe,
                crate::gamedb::display_name(&process.exe, path.as_deref()),
            );
        }
    }
    out
}

/// The monitor (raw HMONITOR) a window (raw HWND) is on, or mostly on.
pub fn monitor_of(window: isize) -> isize {
    unsafe { MonitorFromWindow(HWND(window as *mut _), MONITOR_DEFAULTTONEAREST) }.0 as isize
}

/// The program of the front window.
pub fn foreground_pid() -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid
}

/// True while the process `pid` is still the program `exe`: running, and not a reused id.
pub fn is_running(pid: u32, exe: &str) -> bool {
    use windows::Win32::System::Threading::GetExitCodeProcess;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE;
        let _ = CloseHandle(handle);
        alive && exe_of(pid).is_some_and(|e| e.eq_ignore_ascii_case(exe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_tools_are_not_games() {
        assert!(is_not_game("SnippingTool.exe"));
        assert!(is_not_game("ScreenClippingHost.exe"));
        assert!(is_not_game("ShareX.exe"));
        assert!(!is_not_game("WorldOfWarships64.exe"));
    }

    #[test]
    fn a_running_program_is_found_and_a_wrong_name_is_not() {
        let pid = std::process::id();
        let exe = exe_of(pid).expect("own program name");
        assert!(is_running(pid, &exe));
        assert!(!is_running(pid, "not-this-program.exe"));
        assert!(!is_running(u32::MAX - 3, &exe));
    }
}

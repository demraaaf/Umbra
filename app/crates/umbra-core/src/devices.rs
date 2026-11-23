//! Listing audio devices and the programs currently making sound, for the settings screen.

use anyhow::{Context, Result};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AudioSessionStateExpired, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::core::Interface;

use crate::process::list_processes;

#[derive(Clone, Debug)]
pub struct Microphone {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug)]
pub struct AudioApp {
    pub exe: String,
    /// The program name without `.exe`.
    pub name: String,
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe {
        // Commands run on pool threads that may not have COM yet. An error here just means
        // the thread already has it.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        Ok(CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?)
    }
}

fn take_string(text: windows::core::PWSTR) -> String {
    unsafe {
        let value = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const _));
        value
    }
}

pub fn list_microphones() -> Result<Vec<Microphone>> {
    unsafe {
        let enumerator = enumerator()?;
        let default_id = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .and_then(|d| d.GetId())
            .map(take_string)
            .ok();
        let devices = enumerator
            .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
            .context("cannot list microphones")?;
        let mut list = Vec::new();
        for i in 0..devices.GetCount()? {
            let device = devices.Item(i)?;
            let id = take_string(device.GetId()?);
            let name = device
                .OpenPropertyStore(STGM_READ)
                .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
                .and_then(|value| PropVariantToStringAlloc(&value))
                .map(take_string)
                .unwrap_or_else(|_| id.clone());
            list.push(Microphone {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            });
        }
        // The default first, then alphabetical.
        list.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(list)
    }
}

/// Programs that have an audio session open on the default output device, which is the set
/// of programs whose sound Umbra can capture on its own track.
pub fn list_audio_apps() -> Result<Vec<AudioApp>> {
    unsafe {
        let enumerator = enumerator()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .context("no default output device")?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        let processes = list_processes();
        let mut apps: Vec<AudioApp> = Vec::new();
        for i in 0..sessions.GetCount()? {
            let Ok(control) = sessions.GetSession(i) else {
                continue;
            };
            let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            let Ok(pid) = control.GetProcessId() else {
                continue;
            };
            if pid == 0 || control.IsSystemSoundsSession().0 == 0 {
                continue;
            }
            let Some(process) = processes.iter().find(|p| p.pid == pid) else {
                continue;
            };
            if apps
                .iter()
                .any(|a| a.exe.eq_ignore_ascii_case(&process.exe))
            {
                continue;
            }
            apps.push(AudioApp {
                name: crate::naming::game_name_from_exe(&process.exe),
                exe: process.exe.clone(),
            });
        }
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(apps)
    }
}

/// Process ids that have an audio session open on any active output device: every program
/// that plays sound or has played it and still holds the session. The shared Windows system
/// sounds session (process 0) is left out.
pub fn audio_session_pids() -> Result<Vec<u32>> {
    unsafe {
        let enumerator = enumerator()?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let mut pids = Vec::new();
        for d in 0..devices.GetCount()? {
            let Ok(device) = devices.Item(d) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
            for i in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(control) = sessions.GetSession(i) else {
                    continue;
                };
                if control
                    .GetState()
                    .is_ok_and(|s| s == AudioSessionStateExpired)
                {
                    continue;
                }
                let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                if let Ok(pid) = control.GetProcessId() {
                    if pid != 0 && !pids.contains(&pid) {
                        pids.push(pid);
                    }
                }
            }
        }
        Ok(pids)
    }
}

//! The password guard: UI Automation tells, for each element that gets the keyboard focus,
//! whether it is a password field and whose it is. `inputrules` drops every key while one has
//! focus (and while the guard cannot tell). It listens only while asked to: watching focus
//! makes Chromium programs (Chrome, Edge, Discord) switch on their accessibility support,
//! which costs them a little, so `input` asks only while the program in front keeps keys.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest,
    IUIAutomationElement, IUIAutomationFocusChangedEventHandler,
    IUIAutomationFocusChangedEventHandler_Impl, UIA_IsPasswordPropertyId, UIA_ProcessIdPropertyId,
};
use windows_core::{Interface, Ref, implement};

use crate::inputrules::GuardSays;

/// Longest UI Automation waits for a program before giving up (and the guard says it cannot
/// tell).
const TIMEOUT_MS: u32 = 500;

pub struct Guard {
    wanted: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Guard {
    /// Starts the guard's thread; it listens once `want(true)`. What it learns goes to `tx`,
    /// stamped with the QPC time in seconds.
    pub fn start(tx: Sender<(f64, GuardSays)>) -> Result<Self> {
        let wanted = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (w, s) = (wanted.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("umbra-input-guard".into())
            .spawn(move || {
                if let Err(err) = watch(&w, &s, &tx) {
                    log::warn!("password guard stopped: {err:#}");
                    // Nothing can be told from here on: every key in a program that is not
                    // kept in full is left out.
                    let _ = tx.send((crate::clock::qpc_secs(), GuardSays::Unknown { pid: 0 }));
                }
            })
            .context("cannot start the password guard")?;
        Ok(Self {
            wanted,
            stop,
            thread: Some(thread),
        })
    }

    pub fn want(&self, on: bool) {
        self.wanted.store(on, Ordering::Relaxed);
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(thread) = self.thread.take() else {
            return;
        };
        // UI Automation can hang on a program that does not answer; the thread is then left
        // to end on its own rather than holding up Umbra.
        let deadline = Instant::now() + Duration::from_secs(1);
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

#[implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler {
    tx: Sender<(f64, GuardSays)>,
}

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(
        &self,
        sender: Ref<IUIAutomationElement>,
    ) -> windows_core::Result<()> {
        let says = tell(sender.ok().ok());
        let _ = self.tx.send((crate::clock::qpc_secs(), says));
        Ok(())
    }
}

/// What the cached properties of a focused element say.
fn tell(element: Option<&IUIAutomationElement>) -> GuardSays {
    let Some(element) = element else {
        return GuardSays::Unknown { pid: 0 };
    };
    unsafe {
        let pid = element.CachedProcessId().map_or(0, |p| p.max(0) as u32);
        match element.CachedIsPassword() {
            Ok(password) => GuardSays::Focus {
                pid,
                password: password.as_bool(),
            },
            Err(_) => GuardSays::Unknown { pid },
        }
    }
}

fn watch(wanted: &AtomicBool, stop: &AtomicBool, tx: &Sender<(f64, GuardSays)>) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .context("cannot start COM")?;
    }
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let _com = Com;
    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
            .context("UI Automation is not available")?
    };
    if let Ok(timeouts) = automation.cast::<IUIAutomation2>() {
        unsafe {
            let _ = timeouts.SetConnectionTimeout(TIMEOUT_MS);
            let _ = timeouts.SetTransactionTimeout(TIMEOUT_MS);
        }
    }
    // The two properties come with each event, so reading them asks the program nothing more.
    let cache: IUIAutomationCacheRequest = unsafe { automation.CreateCacheRequest()? };
    unsafe {
        cache.AddProperty(UIA_IsPasswordPropertyId)?;
        cache.AddProperty(UIA_ProcessIdPropertyId)?;
    }
    let handler: IUIAutomationFocusChangedEventHandler = FocusHandler { tx: tx.clone() }.into();
    let mut listening = false;
    while !stop.load(Ordering::Relaxed) {
        let want = wanted.load(Ordering::Relaxed);
        if want && !listening {
            unsafe { automation.AddFocusChangedEventHandler(&cache, &handler)? };
            listening = true;
            // What has focus now: no event comes for it.
            let now = unsafe { automation.GetFocusedElementBuildCache(&cache) }.ok();
            let _ = tx.send((crate::clock::qpc_secs(), tell(now.as_ref())));
        } else if !want && listening {
            unsafe { automation.RemoveFocusChangedEventHandler(&handler)? };
            listening = false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if listening {
        unsafe {
            let _ = automation.RemoveFocusChangedEventHandler(&handler);
        }
    }
    Ok(())
}

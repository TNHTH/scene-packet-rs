//! Windows CLI 控制台重连。

#[cfg(windows)]
pub fn attach_console_if_needed() {
    use windows::Win32::System::Console::{AttachConsole, GetConsoleWindow, ATTACH_PARENT_PROCESS};
    unsafe {
        if GetConsoleWindow().0.is_null() {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(windows))]
pub fn attach_console_if_needed() {}

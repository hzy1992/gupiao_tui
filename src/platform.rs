//! 跨平台兼容代码
//!
//! 主要处理 Windows 控制台中文显示问题。

/// Windows 上启用 UTF-8 代码页，让 stdout 能正确输出中文
#[cfg(windows)]
pub fn enable_utf8_console() {
    use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
    unsafe {
        // 65001 = UTF-8
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
    }
}

#[cfg(not(windows))]
pub fn enable_utf8_console() {}

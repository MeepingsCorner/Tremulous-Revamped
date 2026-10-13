//! The firewall question as a native Windows TaskDialog (dec076).
//!
//! `TaskDialogIndirect` (comctl32 v6) shows the WHY as the main instruction,
//! a second line as content, the details in the footer, and the answers as
//! command links. It is called straight from Rust through a hand-written
//! binding (the one function and two structs; no new crate): the build has no
//! windows-sys/windows crate and this adds none. comctl32 is loaded at run
//! time with LoadLibrary/GetProcAddress, so a missing function (no v6
//! manifest, very old Windows, Wine without it) is a plain `None` and the
//! caller falls back to Godot's own dialog. The Godot exe's manifest
//! already requests Microsoft.Windows.Common-Controls 6.0.0.0, so no
//! activation-context code is needed.
//!
//! The call blocks until the player answers: run it on a thread.

/// The only address the dialog's link opens (https only, fixed).
#[cfg(windows)]
pub const HELP_URL: &str = crate::hoststatus::HELP_URL;

/// What a button answer is: an id (stable meaning) and its label.
pub type Button = (i64, String);

/// Shows the dialog and returns the chosen button id; `default` is the id
/// that has the focus. None when the dialog could not be shown (not Windows,
/// comctl32 v6 missing, the call failed). Closing the window (X, Esc) gives `default`.
pub fn show(
    title: &str,
    main: &str,
    content: &str,
    footer: &str,
    buttons: &[Button],
    default: i64,
) -> Option<i64> {
    #[cfg(windows)]
    {
        win::show(title, main, content, footer, buttons, default)
    }
    #[cfg(not(windows))]
    {
        let _ = (title, main, content, footer, buttons, default);
        None
    }
}

#[cfg(windows)]
mod win {
    use super::Button;
    use std::ffi::c_void;

    const TDF_ENABLE_HYPERLINKS: u32 = 0x0001;
    const TDF_ALLOW_DIALOG_CANCELLATION: u32 = 0x0008;
    /// TDN_HYPERLINK_CLICKED (lParam is the href as UTF-16)
    const TDN_HYPERLINK_CLICKED: u32 = 3;
    const TDF_USE_COMMAND_LINKS_NO_ICON: u32 = 0x0020;
    /// answer ids are offset: 0-2 are Windows' own IDOK/IDCANCEL ids (2 is what Esc and the X return)
    const BASE: i32 = 1000;

    #[repr(C, packed(1))]
    struct TaskDialogButton {
        id: i32,
        text: *const u16,
    }

    /// commctrl.h declares both structs between pshpack1.h and poppack.h
    #[repr(C, packed(1))]
    struct TaskDialogConfig {
        cb_size: u32,
        hwnd_parent: *mut c_void,
        h_instance: *mut c_void,
        flags: u32,
        common_buttons: u32,
        window_title: *const u16,
        main_icon: *const u16,
        main_instruction: *const u16,
        content: *const u16,
        c_buttons: u32,
        buttons: *const TaskDialogButton,
        default_button: i32,
        c_radio_buttons: u32,
        radio_buttons: *const TaskDialogButton,
        default_radio_button: i32,
        verification_text: *const u16,
        expanded_information: *const u16,
        expanded_control_text: *const u16,
        collapsed_control_text: *const u16,
        footer_icon: *const u16,
        footer: *const u16,
        callback: *const c_void,
        callback_data: isize,
        cx_width: u32,
    }

    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
    }

    type TaskDialogIndirect =
        unsafe extern "system" fn(*const TaskDialogConfig, *mut i32, *mut i32, *mut i32) -> i32;

    /// Opens the one fixed help page when its link is clicked; any other
    /// href is ignored (the text of the link is never user data).
    unsafe extern "system" fn callback(
        _hwnd: *mut c_void,
        msg: u32,
        _w: usize,
        lparam: isize,
        _data: isize,
    ) -> i32 {
        if msg == TDN_HYPERLINK_CLICKED && lparam != 0 {
            let mut n = 0usize;
            let p = lparam as *const u16;
            while *p.add(n) != 0 && n < 256 {
                n += 1;
            }
            let href = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            if href == super::HELP_URL {
                open_url(&href);
            }
        }
        0
    }

    /// ShellExecuteW "open" (shell32 loaded at run time, like comctl32).
    unsafe fn open_url(url: &str) {
        let lib = LoadLibraryW(wide("shell32.dll").as_ptr());
        if lib.is_null() {
            return;
        }
        let f = GetProcAddress(lib, b"ShellExecuteW\0".as_ptr());
        if f.is_null() {
            return;
        }
        type Sh = unsafe extern "system" fn(
            *mut c_void,
            *const u16,
            *const u16,
            *const u16,
            *const u16,
            i32,
        ) -> *mut c_void;
        let f: Sh = std::mem::transmute(f);
        f(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        );
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn show(
        title: &str,
        main: &str,
        content: &str,
        footer: &str,
        buttons: &[Button],
        default: i64,
    ) -> Option<i64> {
        // SAFETY: plain Win32 calls; every pointer handed over lives until the call returns
        unsafe {
            let lib = LoadLibraryW(wide("comctl32.dll").as_ptr());
            if lib.is_null() {
                return None;
            }
            let f = GetProcAddress(lib, b"TaskDialogIndirect\0".as_ptr());
            if f.is_null() {
                return None;
            }
            let f: TaskDialogIndirect = std::mem::transmute(f);
            let (t, m, c, ft) = (wide(title), wide(main), wide(content), wide(footer));
            let texts: Vec<Vec<u16>> = buttons.iter().map(|b| wide(&b.1)).collect();
            let bs: Vec<TaskDialogButton> = buttons
                .iter()
                .zip(&texts)
                .map(|(b, w)| TaskDialogButton {
                    id: BASE + b.0 as i32,
                    text: w.as_ptr(),
                })
                .collect();
            let cfg = TaskDialogConfig {
                cb_size: std::mem::size_of::<TaskDialogConfig>() as u32,
                hwnd_parent: std::ptr::null_mut(),
                h_instance: std::ptr::null_mut(),
                flags: TDF_ALLOW_DIALOG_CANCELLATION
                    | TDF_USE_COMMAND_LINKS_NO_ICON
                    | if content.contains("<a href=\"https://") {
                        TDF_ENABLE_HYPERLINKS
                    } else {
                        0
                    },
                common_buttons: 0,
                window_title: t.as_ptr(),
                main_icon: std::ptr::null(),
                main_instruction: m.as_ptr(),
                content: c.as_ptr(),
                c_buttons: bs.len() as u32,
                buttons: bs.as_ptr(),
                default_button: BASE + default as i32,
                c_radio_buttons: 0,
                radio_buttons: std::ptr::null(),
                default_radio_button: 0,
                verification_text: std::ptr::null(),
                expanded_information: std::ptr::null(),
                expanded_control_text: std::ptr::null(),
                collapsed_control_text: std::ptr::null(),
                footer_icon: std::ptr::null(),
                footer: ft.as_ptr(),
                callback: callback as *const c_void,
                callback_data: 0,
                cx_width: 0,
            };
            let mut pressed = 0i32;
            let hr = f(
                &cfg,
                &mut pressed,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if hr < 0 {
                return None;
            }
            Some(if pressed >= BASE {
                (pressed - BASE) as i64
            } else {
                default
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(not(windows))]
    fn not_windows_falls_back() {
        assert_eq!(super::show("t", "m", "c", "f", &[(1, "a".into())], 1), None);
    }

    #[cfg(windows)]
    #[test]
    fn struct_sizes_match_commctrl() {
        // x64, packed(1) as between pshpack1.h and poppack.h: 160 and 12 bytes
        assert_eq!(std::mem::size_of::<super::win::TaskDialogConfig>(), 160);
        assert_eq!(std::mem::size_of::<super::win::TaskDialogButton>(), 12);
    }
}

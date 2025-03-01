//! Clipboard monitoring utility

use error_code::ErrorCode;
use windows_win::{
    raw,
    Window,
    Messages
};

use windows_win::sys::{
    HWND,
    AddClipboardFormatListener,
    RemoveClipboardFormatListener,
    PostMessageW,
    WM_CLIPBOARDUPDATE,
};

const CLOSE_PARAM: isize = -1;

///Shutdown channel
///
///On drop requests shutdown to gracefully close clipboard listener as soon as possible.
///
///This is silently ignored, if there is no thread awaiting
pub struct Shutdown {
    window: HWND,
}

unsafe impl Send for Shutdown {}

impl Drop for Shutdown {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            PostMessageW(self.window, WM_CLIPBOARDUPDATE, 0, CLOSE_PARAM)
        };
    }
}

///Clipboard listener guard.
///
///On drop unsubscribes window from listening on clipboard changes
struct ClipboardListener(HWND);

impl ClipboardListener {
    #[inline]
    ///Subscribes window to clipboard changes.
    pub fn new(window: &Window) -> Result<Self, ErrorCode> {
        let window = window.inner();
        unsafe {
            if AddClipboardFormatListener(window) != 1 {
                Err(ErrorCode::last_system())
            } else {
                Ok(ClipboardListener(window))
            }
        }
    }
}

impl Drop for ClipboardListener {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            RemoveClipboardFormatListener(self.0);
        }
    }
}

///Clipboard monitor
///
///This is implemented via dummy message-only window.
///
///This approach definitely works for console applications,
///but it is not tested on windowed application
///
///Due to nature of implementation, it is not safe to move it into different thread.
///
///If needed, user should create monitor and pass Shutdown handle to the separate thread.
///
///Once created, messages will start accumulating immediately
///
///Therefore you generally should start listening for messages once you created instance
///
///`Monitor` implements `Iterator` by continuously calling `Monitor::recv` and returning the same result.
///This `Iterator` is never ending, even when you perform shutdown.
///
///You should use `Shutdown` to interrupt blocking `Monitor::recv`
pub struct Monitor {
    _listener: ClipboardListener,
    window: Window,
}

impl Monitor {
    #[inline(always)]
    ///Creates new instance
    pub fn new() -> Result<Self, ErrorCode> {
        let window = Window::from_builder(raw::window::Builder::new().class_name("STATIC").parent_message())?;
        let _listener = ClipboardListener::new(&window)?;

        Ok(Self {
            _listener,
            window
        })
    }

    #[inline(always)]
    fn iter(&self) -> Messages {
        let mut msg = Messages::new();
        msg.window(Some(self.window.inner()))
           .low(Some(WM_CLIPBOARDUPDATE))
           .high(Some(WM_CLIPBOARDUPDATE));
        msg
    }

    #[inline(always)]
    ///Creates shutdown channel.
    pub fn shutdown_channel(&self) -> Shutdown {
        Shutdown {
            window: self.window.inner()
        }
    }

    ///Waits for new clipboard message, blocking until then.
    ///
    ///Returns `Ok(true)` if event received.
    ///
    ///If `Shutdown` request detected, then return `Ok(false)`
    pub fn recv(&mut self) -> Result<bool, ErrorCode> {
        for msg in self.iter() {
            let msg = msg?;
            match msg.id() {
                WM_CLIPBOARDUPDATE => return Ok(msg.inner().lParam != CLOSE_PARAM),
                _ => unreachable!(),
            }
        }

        unreachable!();
    }

    ///Attempts to get any clipboard update event
    ///
    ///Returns `Ok(true)` if event received,
    ///otherwise return `Ok(false)` indicating no clipboard event present
    ///
    ///If `Shutdown` request detected, it is ignored
    pub fn try_recv(&mut self) -> Result<bool, ErrorCode> {
        let mut iter = self.iter();
        iter.non_blocking();
        while let Some(msg) = iter.next() {
            let msg = msg?;
            match msg.id() {
                WM_CLIPBOARDUPDATE => {
                    //Skip shutdown requests
                    if msg.inner().lParam == CLOSE_PARAM {
                        continue;
                    }

                    return Ok(true);
                }
                _ => unreachable!(),
            }
        }

        Ok(false)
    }
}

impl Iterator for Monitor {
    type Item = Result<bool, ErrorCode>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.recv())
    }
}

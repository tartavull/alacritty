use std::error::Error;

use block2::RcBlock;
use log::debug;
use objc2::encode::{Encode, Encoding};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, MainThreadMarker};
use objc2_foundation::NSString;
use winit::raw_window_handle::RawWindowHandle;

use alacritty_terminal::grid::Dimensions;

use crate::display::SizeInfo;
use crate::display::window::Window;

#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_pointer_width = "32")]
type CGFloat = f32;
#[cfg(target_pointer_width = "64")]
type CGFloat = f64;

#[repr(C)]
struct CGPoint {
    x: CGFloat,
    y: CGFloat,
}

// SAFETY: The struct is `repr(C)`, and the encoding is correct.
unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[CGFloat::ENCODING, CGFloat::ENCODING]);
}

#[repr(C)]
struct CGSize {
    width: CGFloat,
    height: CGFloat,
}

// SAFETY: The struct is `repr(C)`, and the encoding is correct.
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[CGFloat::ENCODING, CGFloat::ENCODING]);
}

#[repr(C)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

// SAFETY: The struct is `repr(C)`, and the encoding is correct.
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

pub struct WebView {
    view: Retained<AnyObject>,
    last_title: Option<String>,
    last_url: Option<String>,
}

impl WebView {
    pub fn new(window: &Window, size_info: &SizeInfo, url: &str) -> Result<Self, Box<dyn Error>> {
        let _mtm = MainThreadMarker::new().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "WebView must be created on main thread",
            )
        })?;

        let parent = ns_view(window)?;
        let config: *mut AnyObject = unsafe { msg_send![class!(WKWebViewConfiguration), new] };
        let config = unsafe { Retained::from_raw(config) }.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to allocate WKWebViewConfiguration",
            )
        })?;
        let store: *mut AnyObject =
            unsafe { msg_send![class!(WKWebsiteDataStore), defaultDataStore] };
        unsafe {
            let _: () = msg_send![&*config, setWebsiteDataStore: store];
        }

        let frame = webview_frame(window, size_info);
        let view: *mut AnyObject = unsafe { msg_send![class!(WKWebView), alloc] };
        let view: *mut AnyObject =
            unsafe { msg_send![view, initWithFrame: frame, configuration: &*config] };
        let view = unsafe { Retained::from_raw(view) }.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "Failed to init WKWebView")
        })?;

        unsafe {
            let _: () = msg_send![parent, addSubview: &*view];
        }

        let mut web_view = Self { view, last_title: None, last_url: None };
        let initial_url = if url.is_empty() { "about:blank" } else { url };
        web_view.load_url(initial_url);
        Ok(web_view)
    }

    pub fn set_visible(&mut self, visible: bool) {
        unsafe {
            let _: () = msg_send![&*self.view, setHidden: !visible];
        }
    }

    pub fn update_frame(&mut self, window: &Window, size_info: &SizeInfo) {
        let frame = webview_frame(window, size_info);
        unsafe {
            let _: () = msg_send![&*self.view, setFrame: frame];
        }
    }

    pub fn load_url(&mut self, url: &str) -> bool {
        self.last_title = None;
        self.last_url = None;
        let url = NSString::from_str(url);
        let ns_url: *mut AnyObject = unsafe { msg_send![class!(NSURL), URLWithString: &*url] };
        if ns_url.is_null() {
            return false;
        }

        let request: *mut AnyObject =
            unsafe { msg_send![class!(NSURLRequest), requestWithURL: ns_url] };
        let _: *mut AnyObject = unsafe { msg_send![&*self.view, loadRequest: request] };
        true
    }

    pub fn reload(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.view, reload];
        }
    }

    pub fn go_back(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.view, goBack];
        }
    }

    pub fn go_forward(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.view, goForward];
        }
    }

    pub fn exec_js(&mut self, script: &str) {
        self.eval_js_string(script, |_| {});
    }

    pub fn eval_js_string<F>(&mut self, script: &str, callback: F)
    where
        F: FnOnce(Option<String>) + 'static,
    {
        let _mtm = MainThreadMarker::new().expect("WebView JS requires main thread");
        let script = NSString::from_str(script);
        let block = RcBlock::new(move |result: *mut AnyObject, error: *mut AnyObject| {
            if !error.is_null() {
                let error_desc: *mut AnyObject = unsafe { msg_send![error, description] };
                if !error_desc.is_null() {
                    let error_str =
                        unsafe { &*(error_desc as *const NSString) }.to_string();
                    debug!("WebView JS error: {error_str}");
                }
                callback(None);
                return;
            }

            if result.is_null() {
                callback(None);
                return;
            }

            let desc: *mut AnyObject = unsafe { msg_send![result, description] };
            if desc.is_null() {
                callback(None);
                return;
            }

            let output = unsafe { &*(desc as *const NSString) }.to_string();
            callback(Some(output));
        });

        unsafe {
            let _: () =
                msg_send![&*self.view, evaluateJavaScript: &*script, completionHandler: &*block];
        }
    }

    pub fn poll_title(&mut self) -> Option<String> {
        let title: *mut AnyObject = unsafe { msg_send![&*self.view, title] };
        if title.is_null() {
            return None;
        }

        let title = unsafe { &*(title as *const NSString) }.to_string();
        if self.last_title.as_deref() == Some(&title) {
            return None;
        }

        self.last_title = Some(title.clone());
        Some(title)
    }

    pub fn poll_url(&mut self) -> Option<String> {
        let url: *mut AnyObject = unsafe { msg_send![&*self.view, URL] };
        if url.is_null() {
            return None;
        }

        let absolute: *mut AnyObject = unsafe { msg_send![url, absoluteString] };
        if absolute.is_null() {
            return None;
        }

        let url = unsafe { &*(absolute as *const NSString) }.to_string();
        if self.last_url.as_deref() == Some(&url) {
            return None;
        }

        self.last_url = Some(url.clone());
        Some(url)
    }

    pub fn current_url(&self) -> Option<String> {
        let url: *mut AnyObject = unsafe { msg_send![&*self.view, URL] };
        if url.is_null() {
            return None;
        }

        let absolute: *mut AnyObject = unsafe { msg_send![url, absoluteString] };
        if absolute.is_null() {
            return None;
        }

        Some(unsafe { &*(absolute as *const NSString) }.to_string())
    }
}

impl Drop for WebView {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.view, removeFromSuperview];
        }
    }
}

fn ns_view(window: &Window) -> Result<*mut AnyObject, Box<dyn Error>> {
    match window.raw_window_handle() {
        RawWindowHandle::AppKit(handle) => Ok(handle.ns_view.as_ptr() as *mut AnyObject),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "WebView requires an AppKit window",
        )
        .into()),
    }
}

fn webview_frame(window: &Window, size_info: &SizeInfo) -> CGRect {
    let scale_factor = window.scale_factor;
    let x = (f64::from(size_info.padding_x()) / scale_factor) as CGFloat;
    let y = (f64::from(size_info.padding_y()) / scale_factor) as CGFloat;
    let width =
        (f64::from(size_info.width() - size_info.padding_x() - size_info.padding_right())
            / scale_factor) as CGFloat;
    let height =
        (f64::from(size_info.cell_height() * size_info.screen_lines() as f32) / scale_factor)
            as CGFloat;

    CGRect {
        origin: CGPoint { x, y },
        size: CGSize { width, height },
    }
}

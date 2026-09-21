//! WebView2-hosted settings window, created on demand and released on close.

use crate::msg::{UiWaker, WM_APP_SETTINGS_CLOSED, WM_APP_SETTINGS_MSG};
use crate::ui::win::{hinstance, CLASS_SETTINGS};
use crate::util::wide::WStr;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, GetAvailableCoreWebView2BrowserVersionString, ICoreWebView2, ICoreWebView2Controller,
    ICoreWebView2Controller2, ICoreWebView2Environment, COREWEBVIEW2_COLOR,
};
use webview2_com::{CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler, WebMessageReceivedEventHandler};
use windows::core::{Interface, Result, PWSTR};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, SetForegroundWindow, ShowWindow, SW_SHOW, SW_SHOWNORMAL, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
};

struct Inner {
    hwnd: HWND,
    env: Option<ICoreWebView2Environment>,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    queue: VecDeque<String>,
    pending_out: Vec<String>,
    ready: bool,
    waker: UiWaker,
}

pub struct SettingsWindow {
    pub hwnd: HWND,
    inner: Rc<RefCell<Inner>>,
}

pub fn runtime_version() -> Option<String> {
    unsafe {
        let mut p = PWSTR::null();
        GetAvailableCoreWebView2BrowserVersionString(None, &mut p).ok()?;
        if p.is_null() {
            return None;
        }
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

impl SettingsWindow {
    pub fn open(x: i32, y: i32, w: i32, h: i32, dark: bool, user_data: &Path, html: &'static str, waker: UiWaker) -> Result<SettingsWindow> {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                WStr::new(CLASS_SETTINGS).pcwstr(),
                WStr::new(crate::i18n::t("Clipcywin Settings")).pcwstr(),
                WS_OVERLAPPEDWINDOW,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(hinstance()),
                None,
            )?
        };
        unsafe {
            let dark_v = BOOL(dark as i32);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_v as *const _ as *const _, 4);
            let bd = DWMSBT_MAINWINDOW;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &bd as *const _ as *const _, 4);
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(hwnd);
        }
        let inner = Rc::new(RefCell::new(Inner { hwnd, env: None, controller: None, webview: None, queue: VecDeque::new(), pending_out: Vec::new(), ready: false, waker }));
        let win = SettingsWindow { hwnd, inner: inner.clone() };
        win.start_webview(user_data, html, dark)?;
        Ok(win)
    }

    fn start_webview(&self, user_data: &Path, html: &'static str, dark: bool) -> Result<()> {
        let inner = self.inner.clone();
        let ud = WStr::new(&user_data.to_string_lossy());
        let env_handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |hr, env: Option<ICoreWebView2Environment>| {
            hr?;
            let Some(env) = env else { return Ok(()) };
            let hwnd = inner.borrow().hwnd;
            inner.borrow_mut().env = Some(env.clone());
            let inner2 = inner.clone();
            let ctrl_handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |hr, controller: Option<ICoreWebView2Controller>| {
                hr?;
                let Some(controller) = controller else { return Ok(()) };
                unsafe {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    controller.SetBounds(rc)?;
                    if let Ok(c2) = controller.cast::<ICoreWebView2Controller2>() {
                        let _ = c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 0, R: 0, G: 0, B: 0 });
                    }
                    let webview = controller.CoreWebView2()?;
                    if let Ok(s) = webview.Settings() {
                        let _ = s.SetAreDefaultContextMenusEnabled(false);
                        let _ = s.SetIsStatusBarEnabled(false);
                        let _ = s.SetIsZoomControlEnabled(false);
                        let _ = s.SetAreDevToolsEnabled(cfg!(debug_assertions));
                    }
                    let inner3 = inner2.clone();
                    let mut token = 0i64;
                    webview.add_WebMessageReceived(
                        &WebMessageReceivedEventHandler::create(Box::new(move |_wv, args| {
                            if let Some(args) = args {
                                let mut p = PWSTR::null();
                                if args.TryGetWebMessageAsString(&mut p).is_ok() && !p.is_null() {
                                    let s = p.to_string().unwrap_or_default();
                                    CoTaskMemFree(Some(p.0 as *const _));
                                    let waker = {
                                        let mut i = inner3.borrow_mut();
                                        i.queue.push_back(s);
                                        i.waker
                                    };
                                    waker.post(WM_APP_SETTINGS_MSG, 0, 0);
                                }
                            }
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    let page = html.replace("__DARK__", if dark { "dark" } else { "light" });
                    webview.NavigateToString(WStr::new(&page).pcwstr())?;
                    let mut i = inner2.borrow_mut();
                    i.controller = Some(controller);
                    i.webview = Some(webview.clone());
                    i.ready = true;
                    for m in i.pending_out.drain(..) {
                        let _ = webview.PostWebMessageAsJson(WStr::new(&m).pcwstr());
                    }
                }
                Ok(())
            }));
            unsafe { env.CreateCoreWebView2Controller(hwnd, &ctrl_handler)? };
            Ok(())
        }));
        unsafe { CreateCoreWebView2EnvironmentWithOptions(None, ud.pcwstr(), None, &env_handler)? };
        Ok(())
    }

    pub fn drain(&self) -> Vec<String> {
        self.inner.borrow_mut().queue.drain(..).collect()
    }

    pub fn post_json(&self, json: String) {
        let mut i = self.inner.borrow_mut();
        match (&i.webview, i.ready) {
            (Some(wv), true) => unsafe {
                let _ = wv.PostWebMessageAsJson(WStr::new(&json).pcwstr());
            },
            _ => i.pending_out.push(json),
        }
    }

    pub fn resize(&self) {
        let i = self.inner.borrow();
        if let Some(c) = &i.controller {
            unsafe {
                let mut rc = RECT::default();
                let _ = GetClientRect(i.hwnd, &mut rc);
                let _ = c.SetBounds(rc);
            }
        }
    }

    pub fn focus(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// Release the WebView2 controller and destroy the host window.
    pub fn close(self) {
        {
            let mut i = self.inner.borrow_mut();
            if let Some(c) = i.controller.take() {
                unsafe {
                    let _ = c.Close();
                }
            }
            i.webview = None;
            i.env = None;
            i.ready = false;
        }
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
        let waker = self.inner.borrow().waker;
        waker.post(WM_APP_SETTINGS_CLOSED, 0, 0);
    }
}

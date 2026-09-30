//! WebKitGTK host for Roblox's in-app browser requests.
//!
//! WebKitGTK uses the system WebKit libraries and its own web/network
//! processes. The Rusty-blox runtime only supplies Roblox's URL and title; the
//! GTK widget and its event processing stay in this host process.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gtk4::prelude::*;
use webkit6::prelude::*;

struct LoadState {
    cookie_ready: bool,
    pending_uri: Option<String>,
}

pub(crate) struct WebViewHost {
    window: gtk4::Window,
    view: webkit6::WebView,
    load_state: Rc<RefCell<LoadState>>,
    _popup_windows: Rc<RefCell<Vec<gtk4::Window>>>,
}

impl WebViewHost {
    pub(crate) fn new(session_dir: Option<&Path>) -> Result<Self, String> {
        gtk4::init().map_err(|error| format!("initialize GTK for Roblox web view: {error}"))?;

        let network_session = webkit6::NetworkSession::new_ephemeral();
        let view = webkit6::WebView::builder()
            .network_session(&network_session)
            .hexpand(true)
            .vexpand(true)
            .build();

        let window = gtk4::Window::new();
        window.set_default_size(900, 700);
        let header = gtk4::HeaderBar::new();
        let back = gtk4::Button::from_icon_name("go-previous-symbolic");
        let back_view = view.clone();
        back.connect_clicked(move |_| back_view.go_back());
        header.pack_start(&back);
        let forward = gtk4::Button::from_icon_name("go-next-symbolic");
        let forward_view = view.clone();
        forward.connect_clicked(move |_| forward_view.go_forward());
        header.pack_start(&forward);
        let reload = gtk4::Button::from_icon_name("view-refresh-symbolic");
        let reload_view = view.clone();
        reload.connect_clicked(move |_| reload_view.reload());
        header.pack_end(&reload);
        window.set_titlebar(Some(&header));
        window.set_child(Some(&view));
        window.connect_close_request(|window| {
            window.set_visible(false);
            gtk4::glib::Propagation::Stop
        });

        let load_state = Rc::new(RefCell::new(LoadState {
            cookie_ready: false,
            pending_uri: None,
        }));
        let popup_windows = Rc::new(RefCell::new(Vec::new()));
        install_navigation_policy(&view);
        install_popup_handling(&view, &popup_windows);

        if let Some(cookie) = session_dir.and_then(read_roblox_session_cookie) {
            let manager = network_session
                .cookie_manager()
                .ok_or("WebKitGTK did not provide a cookie manager")?;
            let mut session_cookie =
                webkit6::soup::Cookie::new(".ROBLOSECURITY", &cookie, ".roblox.com", "/", -1);
            session_cookie.set_secure(true);
            session_cookie.set_http_only(true);
            let view_after_cookie = view.clone();
            let state_after_cookie = load_state.clone();
            manager.add_cookie(
                &session_cookie,
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if result.is_err() {
                        eprintln!("rusty-blox: could not copy the Roblox session into WebKitGTK");
                    }
                    let mut state = state_after_cookie.borrow_mut();
                    state.cookie_ready = true;
                    if let Some(uri) = state.pending_uri.take() {
                        view_after_cookie.load_uri(&uri);
                    }
                },
            );
        } else {
            load_state.borrow_mut().cookie_ready = true;
        }

        Ok(Self {
            window,
            view,
            load_state,
            _popup_windows: popup_windows,
        })
    }

    pub(crate) fn open(&self, request: roblox_runtime::webview::WebViewRequest) {
        if !webview_uri_allowed(&request.url) {
            eprintln!("rusty-blox: blocked an unsafe Roblox web view URL");
            return;
        }
        eprintln!("rusty-blox: presenting Roblox web view window");
        self.window.set_title(Some(&request.title));
        self.window.present();
        let mut state = self.load_state.borrow_mut();
        if state.cookie_ready {
            self.view.load_uri(&request.url);
        } else {
            state.pending_uri = Some(request.url);
        }
    }

    pub(crate) fn pump_events() {
        if gtk4::glib::MainContext::default().pending() {
            let _ = gtk4::glib::MainContext::default().iteration(false);
        }
    }
}

fn install_navigation_policy(view: &webkit6::WebView) {
    view.connect_decide_policy(|_, decision, kind| {
        if kind != webkit6::PolicyDecisionType::NavigationAction
            && kind != webkit6::PolicyDecisionType::NewWindowAction
        {
            return false;
        }
        let Some(navigation) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else {
            return false;
        };
        let uri = navigation
            .navigation_action()
            .and_then(|action| action.request())
            .and_then(|request| request.uri())
            .map(|uri| uri.to_string())
            .unwrap_or_default();
        if webview_uri_allowed(&uri) {
            decision.use_();
        } else {
            decision.ignore();
        }
        true
    });
}

fn install_popup_handling(
    parent: &webkit6::WebView,
    popup_windows: &Rc<RefCell<Vec<gtk4::Window>>>,
) {
    let popup_windows = popup_windows.clone();
    parent.connect_create(move |parent_view, action| {
        let uri = action
            .request()
            .and_then(|request| request.uri())
            .map(|uri| uri.to_string())
            .unwrap_or_default();
        if !webview_uri_allowed(&uri) {
            eprintln!("rusty-blox: blocked a Roblox web view popup URL");
            return None;
        }

        let popup = webkit6::WebView::builder()
            .related_view(parent_view)
            .hexpand(true)
            .vexpand(true)
            .build();
        install_navigation_policy(&popup);
        install_popup_handling(&popup, &popup_windows);

        let window = gtk4::Window::new();
        window.set_default_size(760, 600);
        window.set_title(Some("Roblox"));
        window.set_child(Some(&popup));
        let windows = popup_windows.clone();
        window.connect_close_request(move |window| {
            window.set_visible(false);
            windows.borrow_mut().retain(|open| open != window);
            gtk4::glib::Propagation::Stop
        });
        let shown_window = window.downgrade();
        let shown_view = popup.clone();
        popup.connect_ready_to_show(move |_| {
            let uri = shown_view
                .uri()
                .map(|uri| uri.to_string())
                .unwrap_or_default();
            if let Some(window) = shown_window.upgrade() {
                if let Some(host) = gtk4::glib::Uri::parse(&uri, gtk4::glib::UriFlags::NONE)
                    .ok()
                    .and_then(|uri| uri.host().map(|host| host.to_string()))
                {
                    window.set_title(Some(&host));
                }
                window.present();
            }
        });
        let closing_window = window.downgrade();
        popup.connect_close(move |_| {
            if let Some(window) = closing_window.upgrade() {
                window.close();
            }
        });
        popup_windows.borrow_mut().push(window);
        Some(popup.upcast())
    });
}

fn webview_uri_allowed(uri: &str) -> bool {
    if uri == "about:blank" {
        return true;
    }
    let Ok(parsed) = gtk4::glib::Uri::parse(uri, gtk4::glib::UriFlags::NONE) else {
        return false;
    };
    parsed.scheme().eq_ignore_ascii_case("https")
        && parsed.host().is_some_and(|host| !host.is_empty())
        && parsed.userinfo().is_none()
        && (parsed.port() == -1 || parsed.port() == 443)
}

fn read_roblox_session_cookie(session_dir: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(session_dir.join("roblox-cookies")).ok()?;
    for line in contents.lines().filter(|line| !line.starts_with('#')) {
        let Some((_, encoded_cookies)) = line.split_once('\t') else {
            continue;
        };
        let cookies = unescape_cookie_line(encoded_cookies);
        for cookie in cookies.split("; ") {
            let Some((name, value)) = cookie.split_once('=') else {
                continue;
            };
            if name == ".ROBLOSECURITY" && !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn unescape_cookie_line(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => output.push('\t'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some(ch) => {
                output.push('\\');
                output.push(ch);
            }
            None => output.push('\\'),
        }
    }
    output
}

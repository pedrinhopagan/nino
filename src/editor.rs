use std::io::Write;

use gtk::glib::{self, ControlFlow, IOCondition};
use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use javascriptcore::ValueExt;
use webkit2gtk::{UserContentManagerExt, WebViewExt};

const PAGE: &str = include_str!("../editor/index.html");
const SCRIPT: &str = include_str!("../editor/dist/editor.js");
pub const CARD: i32 = 680;
pub const HISTORY: i32 = 300;
pub const GAP: i32 = 12;
pub const HEIGHT: i32 = 260;

pub fn run([left, top, width, height]: [i32; 4]) -> Result<(), String> {
    let state = read_line();
    gtk::init().map_err(|e| format!("gtk: {e}"))?;

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace("nino");
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_exclusive_zone(-1);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_layer_shell_margin(Edge::Top, top);
    window.set_layer_shell_margin(Edge::Left, left);
    window.set_size_request(width, height);
    window.set_app_paintable(true);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.connect_destroy(|_| gtk::main_quit());

    let manager = webkit2gtk::UserContentManager::new();
    manager.register_script_message_handler("nino");
    manager.connect_script_message_received(Some("nino"), |_, result| {
        if let Some(value) = result.js_value() {
            let mut out = std::io::stdout();
            let _ = out.write_all(value.to_str().as_bytes());
            let _ = out.write_all(b"\0");
            let _ = out.flush();
        }
    });

    let webview = webkit2gtk::WebView::with_user_content_manager(&manager);
    webview.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    let html = PAGE.replacen("{script}", SCRIPT, 1).replacen("{state}", &state, 1);
    webview.load_html(&html, None);
    window.add(&webview);
    window.show_all();
    webview.grab_focus();

    let mut pending = Vec::new();
    glib::unix_fd_add_local(0, IOCondition::IN | IOCondition::HUP, move |fd, _| {
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            gtk::main_quit();
            return ControlFlow::Break;
        }
        pending.extend_from_slice(&buf[..n as usize]);
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let script = String::from_utf8_lossy(&line);
            webview.run_javascript(&script, None::<&gtk::gio::Cancellable>, |_| {});
        }
        ControlFlow::Continue
    });

    gtk::main();
    Ok(())
}

fn read_line() -> String {
    let mut line = Vec::new();
    let mut byte = 0u8;
    while unsafe { libc::read(0, (&raw mut byte).cast(), 1) } == 1 && byte != b'\n' {
        line.push(byte);
    }
    String::from_utf8_lossy(&line).into_owned()
}

use std::io::Write;

use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use javascriptcore::ValueExt;
use webkit2gtk::{UserContentManagerExt, WebViewExt};

const PAGE: &str = include_str!("../editor/index.html");
const SCRIPT: &str = include_str!("../editor/dist/editor.js");

pub fn run(title: &str, esc: &str) -> Result<(), String> {
    gtk::init().map_err(|e| format!("gtk: {e}"))?;

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace("nino");
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_anchor(Edge::Bottom, true);
    window.set_layer_shell_margin(Edge::Bottom, 72);
    window.set_size_request(640, 220);
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
            let _ = out.flush();
        }
        gtk::main_quit();
    });

    let webview = webkit2gtk::WebView::with_user_content_manager(&manager);
    webview.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    let html = PAGE
        .replace("{title}", title)
        .replace("{esc}", esc)
        .replace("{script}", SCRIPT);
    webview.load_html(&html, None);
    window.add(&webview);
    window.show_all();
    webview.grab_focus();

    gtk::main();
    Ok(())
}

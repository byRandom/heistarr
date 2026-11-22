//! Small shared pieces: icons, copyable values, notices, formatting.

use leptos::prelude::*;

fn icon_svg(name: &str) -> &'static str {
    match name {
        "library" => {
            r#"<rect x="3" y="4" width="18" height="16" rx="1"/><path d="M3 9h18M8 4v5M16 4v5"/>"#
        }
        "plus" => r#"<path d="M12 5v14M5 12h14"/>"#,
        "downloads" => r#"<path d="M12 4v11"/><path d="M7 10l5 5 5-5"/><path d="M5 20h14"/>"#,
        "history" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>"#,
        "settings" => {
            r#"<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1"/>"#
        }
        "info" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>"#,
        "search" => r#"<circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/>"#,
        "copy" => r#"<rect x="8" y="8" width="12" height="12" rx="1"/><path d="M16 8V4H4v12h4"/>"#,
        "check" => r#"<path d="M5 12l5 5 9-10"/>"#,
        "star" => {
            r#"<path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z"/>"#
        }
        "logout" => r#"<path d="M15 4h4v16h-4"/><path d="M10 8l-4 4 4 4"/><path d="M6 12h10"/>"#,
        "trash" => r#"<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>"#,
        _ => "",
    }
}

#[component]
pub fn Icon(name: &'static str, #[prop(optional)] size: Option<u32>) -> impl IntoView {
    let s = size.unwrap_or(18);
    let html = format!(
        r#"<svg width="{s}" height="{s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{}</svg>"#,
        icon_svg(name)
    );
    view! { <span style="display:inline-grid" inner_html=html></span> }
}

/// A value shown so it can be selected, plus a button that copies it.
#[component]
pub fn Copyable(#[prop(into)] text: String) -> impl IntoView {
    let copied = RwSignal::new(false);
    let t = text.clone();
    let copy = move |_| {
        if let Some(w) = web_sys::window() {
            let _ = w.navigator().clipboard().write_text(&t);
            copied.set(true);
            set_timeout(
                move || copied.set(false),
                std::time::Duration::from_millis(1200),
            );
        }
    };
    view! {
        <span class="copy">
            <code>{text}</code>
            <button type="button" title="Copy" aria-label="Copy" on:click=copy>
                {move || if copied.get() { view! { <Icon name="check" size=14/> } } else { view! { <Icon name="copy" size=14/> } }}
            </button>
        </span>
    }
}

/// A message line: error (red), ok, or plain.
#[component]
pub fn Notice(msg: RwSignal<Option<(bool, String)>>) -> impl IntoView {
    move || {
        msg.get().map(|(ok, text)| {
            let class = if ok { "notice ok" } else { "notice error" };
            view! { <div class=class role="status">{text}</div> }
        })
    }
}

pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// `2026-11-22 19:52` in the browser's time zone.
pub fn time(ts: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

pub fn status_label(s: heistarr_core::api::MovieStatus) -> &'static str {
    use heistarr_core::api::MovieStatus::*;
    match s {
        Wanted => "Wanted",
        Downloading => "Downloading",
        Available => "Available",
    }
}

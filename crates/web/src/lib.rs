//! Heistarr web UI: a Leptos app compiled to WebAssembly and served by the
//! Heistarr server, talking to it through `/api`.

mod api;
mod pages;
mod settings;
mod ui;

use heistarr_core::api::{Me, SetupState};
use leptos::prelude::*;
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::path;
use std::cell::Cell;

#[derive(Clone, Debug, PartialEq)]
pub enum Auth {
    Loading,
    NeedsOwner,
    SignedOut,
    SignedIn(Me),
}

thread_local! {
    static AUTH: Cell<Option<RwSignal<Auth>>> = const { Cell::new(None) };
}

/// Called by the API layer on a 401: back to the sign-in screen.
pub fn session_lost() {
    if let Some(a) = AUTH.with(Cell::get)
        && matches!(a.get_untracked(), Auth::SignedIn(_))
    {
        a.set(Auth::SignedOut);
    }
}

/// The sign-in state. Works outside the component tree (async tasks).
pub fn auth() -> RwSignal<Auth> {
    AUTH.with(Cell::get)
        .expect("auth signal is set when the app starts")
}

pub const THEMES: [(&str, &str, &str); 6] = [
    ("miami", "Miami Neon", "#ff3ea5"),
    ("chunkscope", "Chunkscope", "#03bc82"),
    ("neon-noir", "Neon Noir", "#ff2e88"),
    ("hazard", "Hazard", "#f5e14b"),
    ("synthwave", "Synthwave", "#ff5fcf"),
    ("terminal", "Terminal", "#39ff88"),
];

/// Applies a theme to the page and remembers it in this browser.
pub fn apply_theme(id: &str) {
    let id = if THEMES.iter().any(|t| t.0 == id) {
        id
    } else {
        "miami"
    };
    if let Some(w) = web_sys::window() {
        if let Some(el) = w.document().and_then(|d| d.document_element()) {
            let _ = el.set_attribute("data-theme", id);
        }
        if let Ok(Some(s)) = w.local_storage() {
            let _ = s.set_item("heistarr-theme", id);
        }
    }
}

fn stored_theme() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("heistarr-theme").ok().flatten())
        .unwrap_or_else(|| "miami".into())
}

async fn load_auth(a: RwSignal<Auth>) {
    match api::get::<Me>("/me").await {
        Ok(me) => a.set(Auth::SignedIn(me)),
        Err(_) => match api::get::<SetupState>("/setup").await {
            Ok(s) if s.needs_owner => a.set(Auth::NeedsOwner),
            _ => a.set(Auth::SignedOut),
        },
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    apply_theme(&stored_theme());
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let a = RwSignal::new(Auth::Loading);
    AUTH.with(|c| c.set(Some(a)));
    leptos::task::spawn_local(load_auth(a));

    move || match a.get() {
        Auth::Loading => {
            view! { <div class="gate"><p class="weak">"Loading…"</p></div> }.into_any()
        }
        Auth::NeedsOwner => view! { <pages::Gate first_run=true/> }.into_any(),
        Auth::SignedOut => view! { <pages::Gate first_run=false/> }.into_any(),
        Auth::SignedIn(me) => view! { <Shell me=me/> }.into_any(),
    }
}

#[component]
fn Shell(me: Me) -> impl IntoView {
    let initial = me
        .username
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let role = if me.is_owner { "Owner" } else { "User" };
    let logout = move |_| {
        leptos::task::spawn_local(async move {
            let _ = api::post_unit("/logout", &()).await;
            auth().set(Auth::SignedOut);
        });
    };
    view! {
        <Router>
            <div class="shell">
                <nav class="side nav" aria-label="Main">
                    <div class="brand">
                        <div class="logo" aria-hidden="true"><span></span><span></span><span></span><span></span></div>
                        <span class="head">"Heistarr"</span>
                    </div>
                    <A href="/" exact=true><ui::Icon name="library"/><span>"Movies"</span></A>
                    <A href="/add"><ui::Icon name="plus"/><span>"Add movie"</span></A>
                    <A href="/downloads"><ui::Icon name="downloads"/><span>"Downloads"</span></A>
                    <A href="/history"><ui::Icon name="history"/><span>"History"</span></A>
                    <A href="/settings"><ui::Icon name="settings"/><span>"Settings"</span></A>
                    <A href="/about"><ui::Icon name="info"/><span>"About"</span></A>
                    <div class="user">
                        <div class="avatar" aria-hidden="true">{initial}</div>
                        <div class="col" style="gap:0;flex:1;min-width:0">
                            <span style="font-weight:600;font-size:13px" class="ellipsis">{me.username.clone()}</span>
                            <span class="small weak">{role}</span>
                        </div>
                        <button class="btn small icon-btn" title="Sign out" aria-label="Sign out" on:click=logout>
                            <ui::Icon name="logout"/>
                        </button>
                    </div>
                </nav>
                <main class="main grid-bg">
                    <Routes fallback=|| view! { <div class="page"><h1>"Not found"</h1></div> }>
                        <Route path=path!("/") view=pages::Movies/>
                        <Route path=path!("/add") view=pages::AddMovie/>
                        <Route path=path!("/movies/:id") view=pages::MovieDetailPage/>
                        <Route path=path!("/downloads") view=pages::Downloads/>
                        <Route path=path!("/history") view=pages::History/>
                        <Route path=path!("/settings") view=settings::SettingsPage/>
                        <Route path=path!("/about") view=pages::About/>
                    </Routes>
                </main>
            </div>
        </Router>
    }
}

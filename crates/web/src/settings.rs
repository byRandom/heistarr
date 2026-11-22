//! The Settings screen.

use crate::api;
use crate::ui::{Icon, Notice};
use heistarr_core::api::{Indexer, Settings, TestResult};
use heistarr_core::release::{Language, Resolution, Source};
use heistarr_core::rules::Quality;
use leptos::prelude::*;

type Msg = RwSignal<Option<(bool, String)>>;
type GetFn = fn(&Settings) -> String;
type SetFn = fn(&mut Settings, String);

fn field(
    s: RwSignal<Settings>,
    label: &'static str,
    kind: &'static str,
    placeholder: &'static str,
    get: GetFn,
    set: SetFn,
) -> impl IntoView {
    view! {
        <label class="field">
            <span>{label}</span>
            <input type=kind placeholder=placeholder autocomplete="off" spellcheck="false"
                prop:value=move || s.with(get)
                on:input=move |ev| s.update(|x| set(x, event_target_value(&ev)))/>
        </label>
    }
}

fn check(
    s: RwSignal<Settings>,
    label: &'static str,
    get: fn(&Settings) -> bool,
    set: fn(&mut Settings, bool),
) -> impl IntoView {
    view! {
        <label class="check">
            <input type="checkbox" prop:checked=move || s.with(get)
                on:change=move |ev| s.update(|x| set(x, event_target_checked(&ev)))/>
            {label}
        </label>
    }
}

fn words(v: &[String]) -> String {
    v.join(", ")
}

fn parse_words(t: &str) -> Vec<String> {
    t.split([',', '\n'])
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

fn res_key(r: Resolution) -> String {
    serde_json::to_value(r)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn res_from(k: &str) -> Resolution {
    serde_json::from_value(serde_json::Value::String(k.into())).unwrap_or(Resolution::R1080)
}

fn src_key(s: Source) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn src_from(k: &str) -> Source {
    serde_json::from_value(serde_json::Value::String(k.into())).unwrap_or(Source::BluRay)
}

fn res_select(
    s: RwSignal<Settings>,
    label: &'static str,
    get: fn(&Settings) -> Resolution,
    set: fn(&mut Settings, Resolution),
) -> impl IntoView {
    view! {
        <label class="field">
            <span>{label}</span>
            <select on:change=move |ev| s.update(|x| set(x, res_from(&event_target_value(&ev))))>
                {Resolution::ALL.into_iter().map(|r| {
                    let k = res_key(r);
                    view! { <option value=k selected=move || s.with(get) == r>{r.label()}</option> }
                }).collect_view()}
            </select>
        </label>
    }
}

const SOURCES: [Source; 6] = [
    Source::Dvd,
    Source::Hdtv,
    Source::WebRip,
    Source::WebDl,
    Source::BluRay,
    Source::Remux,
];

async fn run_test(path: String, s: Settings, out: Msg) {
    out.set(Some((true, "Testing…".into())));
    match api::post::<_, TestResult>(&path, &s).await {
        Ok(r) => out.set(Some((r.ok, r.message))),
        Err(e) => out.set(Some((false, e.message))),
    }
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let loaded = LocalResource::new(|| api::get::<Settings>("/settings"));
    view! {
        <div class="page">
            <h1>"Settings"</h1>
            {move || match loaded.get() {
                None => view! { <p class="weak">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="notice error">{e.message}</div> }.into_any(),
                Some(Ok(s)) => view! { <SettingsForm initial=s/> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn SettingsForm(initial: Settings) -> impl IntoView {
    let s = RwSignal::new(initial);
    let saved: Msg = RwSignal::new(None);
    let t_tmdb: Msg = RwSignal::new(None);
    let t_client: Msg = RwSignal::new(None);
    let t_media: Msg = RwSignal::new(None);
    let preferred = RwSignal::new(s.with_untracked(|x| words(&x.rules.preferred_words)));
    let blocked = RwSignal::new(s.with_untracked(|x| words(&x.rules.blocked_words)));

    let save = move |_| {
        s.update(|x| {
            x.rules.preferred_words = parse_words(&preferred.get_untracked());
            x.rules.blocked_words = parse_words(&blocked.get_untracked());
        });
        let body = s.get_untracked();
        leptos::task::spawn_local(async move {
            match api::put::<_, Settings>("/settings", &body).await {
                Ok(n) => {
                    s.set(n);
                    saved.set(Some((true, "Saved.".into())));
                }
                Err(e) => saved.set(Some((false, e.message))),
            }
        });
    };
    let test = move |what: &'static str, out: Msg| {
        let body = s.get_untracked();
        leptos::task::spawn_local(run_test(format!("/settings/test/{what}"), body, out));
    };
    let is_synology = move || s.with(|x| x.download_client == "synology");

    view! {
        <section class="panel">
            <h2>"Appearance"</h2>
            <label class="field" style="max-width:320px">
                <span>"Theme"</span>
                <select on:change=move |ev| {
                    let v = event_target_value(&ev);
                    crate::apply_theme(&v);
                    s.update(|x| x.theme = v);
                }>
                    {crate::THEMES.into_iter().map(|(id, name, _)| view! {
                        <option value=id selected=move || s.with(|x| x.theme == id || (x.theme.is_empty() && id == "miami"))>{name}</option>
                    }).collect_view()}
                </select>
            </label>
        </section>

        <section class="panel">
            <h2>"Movie information (TMDB)"</h2>
            <p class="hint">"Heistarr uses your own key from "
                <a href="https://www.themoviedb.org/settings/api" target="_blank" rel="noopener noreferrer">"themoviedb.org/settings/api"</a>
                ": the API key (v3) or the API Read Access Token both work."</p>
            <div class="row">
                {field(s, "TMDB key", "password", "", |x| x.tmdb_key.clone(), |x, v| x.tmdb_key = v)}
                {field(s, "Language for titles and overviews", "text", "en-US, es-ES, es-MX…", |x| x.metadata_language.clone(), |x, v| x.metadata_language = v)}
            </div>
            <div class="row">
                <button class="btn" on:click=move |_| test("tmdb", t_tmdb)>"Test"</button>
                <Notice msg=t_tmdb/>
            </div>
        </section>

        <section class="panel">
            <h2>"Library"</h2>
            {field(s, "Movies folder", "text", "/data/media/movies", |x| x.movies_root.clone(), |x, v| x.movies_root = v)}
            <p class="hint">"Imported movies go to "<code>"Movies folder/Title (Year)/Title (Year).ext"</code>
                ". Put downloads and the library on the same volume so Heistarr can hardlink instead of copying."</p>
        </section>

        <section class="panel">
            <h2>"Download client"</h2>
            <div class="row">
                <label class="check">
                    <input type="radio" name="client" prop:checked=move || !is_synology()
                        on:change=move |_| s.update(|x| x.download_client = "qbittorrent".into())/>
                    "qBittorrent"
                </label>
                <label class="check">
                    <input type="radio" name="client" prop:checked=is_synology
                        on:change=move |_| s.update(|x| x.download_client = "synology".into())/>
                    "Synology Download Station"
                </label>
            </div>
            {move || if is_synology() {
                view! {
                    <div class="row">
                        {field(s, "DSM address", "url", "http://nas.local:5000", |x| x.synology_url.clone(), |x, v| x.synology_url = v)}
                        {field(s, "Username", "text", "", |x| x.synology_username.clone(), |x, v| x.synology_username = v)}
                        {field(s, "Password", "password", "", |x| x.synology_password.clone(), |x, v| x.synology_password = v)}
                    </div>
                    {field(s, "Destination shared folder", "text", "data/torrents", |x| x.synology_destination.clone(), |x, v| x.synology_destination = v)}
                    <p class="hint">"A shared folder path as Download Station shows it, without "<code>"/volume1"</code>
                        ". Use an account without two-step verification and with Download Station access."
                        " Removing a finished task in Download Station keeps its files."</p>
                }.into_any()
            } else {
                view! {
                    <div class="row">
                        {field(s, "Web UI address", "url", "http://qbittorrent:8080", |x| x.qbit_url.clone(), |x, v| x.qbit_url = v)}
                        {field(s, "Username", "text", "", |x| x.qbit_username.clone(), |x, v| x.qbit_username = v)}
                        {field(s, "Password", "password", "", |x| x.qbit_password.clone(), |x, v| x.qbit_password = v)}
                    </div>
                    {field(s, "Category", "text", "heistarr", |x| x.qbit_category.clone(), |x, v| x.qbit_category = v)}
                }.into_any()
            }}
            <h3>"Path mapping"</h3>
            <p class="hint">"Only when the download client sees the files under another path than Heistarr (for example "
                <code>"/volume1/data/torrents"</code>" on the NAS and "<code>"/data/torrents"</code>" in the Heistarr container). "
                "For Download Station the client path is "<code>"/"</code>" + the shared folder, e.g. "<code>"/data/torrents"</code>"."</p>
            <div class="row">
                {field(s, "Client path", "text", "/data/torrents", |x| x.remote_path.clone(), |x, v| x.remote_path = v)}
                {field(s, "Same folder here", "text", "/data/torrents", |x| x.local_path.clone(), |x, v| x.local_path = v)}
            </div>
            {check(s, "Remove the download from the client after import", |x| x.remove_after_import, |x, v| x.remove_after_import = v)}
            <div class="row">
                <button class="btn" on:click=move |_| test("client", t_client)>"Test"</button>
                <Notice msg=t_client/>
            </div>
        </section>

        <section class="panel">
            <h2>"Media server"</h2>
            <label class="field" style="max-width:320px">
                <span>"Tell this server to rescan after an import"</span>
                <select on:change=move |ev| { let v = event_target_value(&ev); s.update(|x| x.media_server_kind = v) }>
                    {[("", "None"), ("jellyfin", "Jellyfin"), ("emby", "Emby"), ("plex", "Plex")].into_iter().map(|(k, n)| view! {
                        <option value=k selected=move || s.with(|x| x.media_server_kind == k)>{n}</option>
                    }).collect_view()}
                </select>
            </label>
            {move || s.with(|x| !x.media_server_kind.is_empty()).then(|| view! {
                <div class="row">
                    {field(s, "Address", "url", "http://jellyfin:8096", |x| x.media_server_url.clone(), |x, v| x.media_server_url = v)}
                    {field(s, "API key / token", "password", "", |x| x.media_server_token.clone(), |x, v| x.media_server_token = v)}
                </div>
                <div class="row">
                    <button class="btn" on:click=move |_| test("media", t_media)>"Test"</button>
                    <Notice msg=t_media/>
                </div>
            })}
        </section>

        <section class="panel">
            <h2>"Release rules"</h2>
            <div class="row">
                {res_select(s, "Minimum resolution", |x| x.rules.min_resolution, |x, v| x.rules.min_resolution = v)}
                {res_select(s, "Maximum resolution", |x| x.rules.max_resolution, |x, v| x.rules.max_resolution = v)}
                {field(s, "Minimum size (MiB)", "number", "0 = no limit", |x| x.rules.min_size_mib.to_string(), |x, v| x.rules.min_size_mib = v.parse().unwrap_or(0))}
                {field(s, "Maximum size (MiB)", "number", "0 = no limit", |x| x.rules.max_size_mib.to_string(), |x, v| x.rules.max_size_mib = v.parse().unwrap_or(0))}
                {field(s, "Minimum seeders", "number", "", |x| x.rules.min_seeders.to_string(), |x, v| x.rules.min_seeders = v.parse().unwrap_or(0))}
            </div>
            <div class="row">
                <label class="field">
                    <span>"Preferred words (comma separated, +15 each)"</span>
                    <input type="text" prop:value=move || preferred.get() on:input=move |ev| preferred.set(event_target_value(&ev))/>
                </label>
                <label class="field">
                    <span>"Blocked words (reject the release)"</span>
                    <input type="text" prop:value=move || blocked.get() on:input=move |ev| blocked.set(event_target_value(&ev))/>
                </label>
            </div>
            {check(s, "Allow x265 / HEVC", |x| x.rules.allow_x265, |x, v| x.rules.allow_x265 = v)}
            <h3>"Audio language"</h3>
            <p class="hint">"Tick none to accept any language. A release without a language tag counts as English."</p>
            <div class="row">
                {Language::ALL.into_iter().filter(|l| !matches!(l, Language::Multi | Language::Dual)).map(|l| view! {
                    <label class="check">
                        <input type="checkbox" prop:checked=move || s.with(|x| x.rules.languages.contains(&l))
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                s.update(|x| {
                                    x.rules.languages.retain(|y| *y != l);
                                    if on { x.rules.languages.push(l); }
                                });
                            }/>
                        {l.label()}
                    </label>
                }).collect_view()}
            </div>
            {check(s, "MULTi and DUAL releases count as a wanted language", |x| x.rules.accept_multi, |x, v| x.rules.accept_multi = v)}
            <h3>"Upgrades"</h3>
            {check(s, "Replace the file when a better release appears", |x| x.rules.upgrades, |x, v| x.rules.upgrades = v)}
            <div class="row">
                {res_select(s, "Stop upgrading at resolution", |x| x.rules.cutoff.resolution, |x, v| x.rules.cutoff = Quality { resolution: v, ..x.rules.cutoff })}
                <label class="field">
                    <span>"… and source"</span>
                    <select on:change=move |ev| { let v = src_from(&event_target_value(&ev)); s.update(|x| x.rules.cutoff.source = v) }>
                        {SOURCES.into_iter().map(|src| view! {
                            <option value=src_key(src) selected=move || s.with(|x| x.rules.cutoff.source == src)>{src.label()}</option>
                        }).collect_view()}
                    </select>
                </label>
            </div>
        </section>

        <div class="row" style="position:sticky;bottom:0;padding:12px 0;background:var(--bg)">
            <button class="btn primary" on:click=save>"Save settings"</button>
            <Notice msg=saved/>
        </div>

        <Indexers/>
    }
}

#[component]
fn Indexers() -> impl IntoView {
    let list = LocalResource::new(|| api::get::<Vec<Indexer>>("/indexers"));
    let editing: RwSignal<Option<Indexer>> = RwSignal::new(None);
    let note: Msg = RwSignal::new(None);

    let save = move |_| {
        let Some(ix) = editing.get_untracked() else {
            return;
        };
        leptos::task::spawn_local(async move {
            match api::post::<_, Indexer>("/indexers", &ix).await {
                Ok(_) => {
                    editing.set(None);
                    note.set(Some((true, "Indexer saved.".into())));
                    list.refetch();
                }
                Err(e) => note.set(Some((false, e.message))),
            }
        });
    };
    let test = move |_| {
        let Some(ix) = editing.get_untracked() else {
            return;
        };
        note.set(Some((true, "Testing…".into())));
        leptos::task::spawn_local(async move {
            match api::post::<_, TestResult>("/indexers/test", &ix).await {
                Ok(r) => note.set(Some((r.ok, r.message))),
                Err(e) => note.set(Some((false, e.message))),
            }
        });
    };
    let remove = move |id: i64| {
        leptos::task::spawn_local(async move {
            let _ = api::delete(&format!("/indexers/{id}")).await;
            list.refetch();
        });
    };
    let edit_field = move |label: &'static str,
                           kind: &'static str,
                           placeholder: &'static str,
                           get: fn(&Indexer) -> String,
                           set: fn(&mut Indexer, String)| {
        view! {
            <label class="field">
                <span>{label}</span>
                <input type=kind placeholder=placeholder autocomplete="off" spellcheck="false"
                    prop:value=move || editing.with(|e| e.as_ref().map(get).unwrap_or_default())
                    on:input=move |ev| editing.update(|e| if let Some(e) = e { set(e, event_target_value(&ev)) })/>
            </label>
        }
    };

    view! {
        <section class="panel">
            <div class="row spread">
                <h2>"Indexers (Torznab)"</h2>
                <button class="btn" on:click=move |_| editing.set(Some(Indexer { enabled: true, ..Default::default() }))>
                    <Icon name="plus" size=16/>"Add indexer"
                </button>
            </div>
            <p class="hint">"Jackett: copy an indexer's Torznab feed URL. Prowlarr: "<code>"http://prowlarr:9696/<id>/"</code>". Paste the API key from the same app."</p>
            {move || match list.get() {
                None => view! { <p class="weak">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="notice error">{e.message}</div> }.into_any(),
                Some(Ok(v)) if v.is_empty() => view! { <p class="weak">"No indexers yet."</p> }.into_any(),
                Some(Ok(v)) => view! {
                    <div class="list">
                        {v.into_iter().map(|ix| {
                            let id = ix.id;
                            let e = ix.clone();
                            view! {
                                <div class="item">
                                    <div class="grow col" style="gap:2px">
                                        <b>{ix.name.clone()}{(!ix.enabled).then_some(" (disabled)")}</b>
                                        <span class="mono small weak ellipsis">{ix.url.clone()}</span>
                                    </div>
                                    <button class="btn small" on:click=move |_| editing.set(Some(e.clone()))>"Edit"</button>
                                    <button class="btn small danger" on:click=move |_| remove(id)>"Delete"</button>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }.into_any(),
            }}
            {move || editing.with(|e| e.is_some()).then(|| view! {
                <div class="panel" style="background:var(--field)">
                    <div class="row">
                        {edit_field("Name", "text", "Jackett", |i| i.name.clone(), |i, v| i.name = v)}
                        {edit_field("Torznab URL", "url", "http://jackett:9117/api/v2.0/indexers/all/results/torznab/", |i| i.url.clone(), |i, v| i.url = v)}
                        {edit_field("API key", "password", "", |i| i.api_key.clone(), |i, v| i.api_key = v)}
                    </div>
                    <label class="check">
                        <input type="checkbox" prop:checked=move || editing.with(|e| e.as_ref().is_some_and(|e| e.enabled))
                            on:change=move |ev| editing.update(|e| if let Some(e) = e { e.enabled = event_target_checked(&ev) })/>
                        "Enabled"
                    </label>
                    <div class="row">
                        <button class="btn primary" on:click=save>"Save"</button>
                        <button class="btn" on:click=test>"Test"</button>
                        <button class="btn" on:click=move |_| editing.set(None)>"Cancel"</button>
                    </div>
                </div>
            })}
            <Notice msg=note/>
        </section>
    }
}

//! Screens: sign-in, movies, add, movie detail, downloads, history, about.

use crate::api;
use crate::ui::{self, Copyable, Icon, Notice};
use crate::{Auth, auth};
use heistarr_core::api::*;
use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

type Msg = RwSignal<Option<(bool, String)>>;

fn msg() -> Msg {
    RwSignal::new(None)
}

// ---------------------------------------------------------------- sign-in

#[component]
pub fn Gate(first_run: bool) -> impl IntoView {
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = msg();
    let busy = RwSignal::new(false);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        let body = Login {
            username: username.get_untracked(),
            password: password.get_untracked(),
        };
        leptos::task::spawn_local(async move {
            let path = if first_run { "/setup" } else { "/login" };
            match api::post::<_, Me>(path, &body).await {
                Ok(me) => auth().set(Auth::SignedIn(me)),
                Err(e) => error.set(Some((false, e.message))),
            }
            busy.set(false);
        });
    };
    view! {
        <div class="gate">
            <div class="sun" aria-hidden="true"></div>
            <div class="horizon" aria-hidden="true"></div>
            <form class="panel" on:submit=submit>
                <div class="brand" style="padding:0">
                    <div class="logo" aria-hidden="true"><span></span><span></span><span></span><span></span></div>
                    <h1>"Heistarr"</h1>
                </div>
                {if first_run {
                    view! {
                        <p>"Welcome. Create the owner account: it can change every setting."</p>
                    }.into_any()
                } else {
                    view! { <p class="weak">"Sign in to continue."</p> }.into_any()
                }}
                <label class="field">
                    <span>"Username"</span>
                    <input type="text" autocomplete="username" required
                        prop:value=move || username.get()
                        on:input=move |ev| username.set(event_target_value(&ev))/>
                </label>
                <label class="field">
                    <span>"Password"</span>
                    <input type="password" required
                        autocomplete=if first_run { "new-password" } else { "current-password" }
                        minlength=if first_run { "8" } else { "1" }
                        prop:value=move || password.get()
                        on:input=move |ev| password.set(event_target_value(&ev))/>
                </label>
                {first_run.then(|| view! { <p class="hint">"At least 8 characters."</p> })}
                <Notice msg=error/>
                <button class="btn primary" type="submit" disabled=move || busy.get()>
                    {if first_run { "Create owner" } else { "Sign in" }}
                </button>
            </form>
        </div>
    }
}

// ---------------------------------------------------------------- movies

/// Calls `f` every `secs` seconds while the current view is shown.
fn every(secs: u64, f: impl Fn() + 'static) {
    let handle = set_interval_with_handle(f, std::time::Duration::from_secs(secs)).ok();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });
}

#[component]
fn PosterCard(
    href: String,
    title: String,
    year: Option<i32>,
    poster: Option<String>,
    #[prop(optional)] status: Option<MovieStatus>,
) -> impl IntoView {
    let img = api::poster_url(&poster, "w342");
    let has_img = img.is_some();
    let t = title.clone();
    let tooltip = title.clone();
    view! {
        <A href=href attr:class="poster-card">
            <div class="poster">
                {img.map(|src| view! { <img src=src alt="" loading="lazy"/> })}
                {(!has_img).then(|| view! { <div class="ph">{t}</div> })}
                {status.map(|s| {
                    let class = format!("chip {}", s.as_str());
                    view! { <span class=class>{ui::status_label(s)}</span> }
                })}
            </div>
            <div class="caption">
                <span title=tooltip>{title}</span>
                <span class="mono">{year.map(|y| y.to_string()).unwrap_or_default()}</span>
            </div>
        </A>
    }
}

#[component]
pub fn Movies() -> impl IntoView {
    let filter = RwSignal::new(String::new());
    let movies = LocalResource::new(|| api::get::<Vec<Movie>>("/movies"));
    every(15, move || movies.refetch());
    let scan_msg = msg();
    let scanning = RwSignal::new(false);
    let scan = move |_| {
        scanning.set(true);
        scan_msg.set(Some((true, "Scanning the movies folder…".into())));
        leptos::task::spawn_local(async move {
            match api::post::<_, ScanReport>("/library/scan", &()).await {
                Ok(r) => {
                    let mut t = format!("{} added, {} already known.", r.matched.len(), r.already);
                    if !r.unmatched.is_empty() {
                        t.push_str(&format!(" Not recognised: {}.", r.unmatched.join(", ")));
                    }
                    scan_msg.set(Some((true, t)));
                    movies.refetch();
                }
                Err(e) => scan_msg.set(Some((false, e.message))),
            }
            scanning.set(false);
        });
    };
    view! {
        <div class="page">
            <div class="row spread">
                <h1>"Movies"</h1>
                <div class="row">
                    <input type="search" placeholder="Filter…" aria-label="Filter movies" style="width:220px"
                        prop:value=move || filter.get() on:input=move |ev| filter.set(event_target_value(&ev))/>
                    <button class="btn" on:click=scan disabled=move || scanning.get()
                        title="Add the movies already in your movies folder">"Scan library"</button>
                    <A href="/add" attr:class="btn primary"><Icon name="plus" size=16/>"Add movie"</A>
                </div>
            </div>
            <Notice msg=scan_msg/>
            {move || match movies.get() {
                None => view! { <p class="weak">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="notice error">{e.message}</div> }.into_any(),
                Some(Ok(list)) if list.is_empty() => view! {
                    <div class="panel">
                        <h2>"Your library is empty"</h2>
                        <p>"Add a movie, or use "<b>"Scan library"</b>" to bring in the movies you already have. Set up TMDB, an indexer and your download client in "<A href="/settings">"Settings"</A>" first."</p>
                    </div>
                }.into_any(),
                Some(Ok(list)) => {
                    let f = filter.get().to_lowercase();
                    let shown: Vec<Movie> = list.into_iter().filter(|m| f.is_empty() || m.title.to_lowercase().contains(&f)).collect();
                    let counts = if shown.len() == 1 { "1 movie".to_string() } else { format!("{} movies", shown.len()) };
                    view! {
                        <p class="small weak">{counts}</p>
                        <div class="posters">
                            {shown.into_iter().map(|m| view! {
                                <PosterCard href=format!("/movies/{}", m.id) title=m.title year=m.year poster=m.poster_path status=m.status/>
                            }).collect_view()}
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}

// ---------------------------------------------------------------- add

#[component]
pub fn AddMovie() -> impl IntoView {
    let query = RwSignal::new(String::new());
    let results: RwSignal<Option<Vec<CatalogMovie>>> = RwSignal::new(None);
    let search_now = RwSignal::new(true);
    let note = msg();
    let busy = RwSignal::new(false);
    let navigate = use_navigate();

    let search = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let q = query.get_untracked();
        if q.trim().is_empty() {
            return;
        }
        busy.set(true);
        note.set(None);
        leptos::task::spawn_local(async move {
            match api::get::<Vec<CatalogMovie>>(&format!("/catalog/search?q={}", api::enc(&q)))
                .await
            {
                Ok(r) => results.set(Some(r)),
                Err(e) => note.set(Some((false, e.message))),
            }
            busy.set(false);
        });
    };

    let add = move |tmdb_id: i64| {
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            let body = heistarr_core::api::AddMovie {
                tmdb_id,
                search_now: search_now.get_untracked(),
            };
            match api::post::<_, Movie>("/movies", &body).await {
                Ok(m) => navigate(&format!("/movies/{}", m.id), Default::default()),
                Err(e) => note.set(Some((false, e.message))),
            }
        });
    };

    view! {
        <div class="page">
            <h1>"Add movie"</h1>
            <form class="row" on:submit=search role="search">
                <input type="search" placeholder="Search The Movie Database…" aria-label="Movie title" style="flex:1 1 320px"
                    prop:value=move || query.get() on:input=move |ev| query.set(event_target_value(&ev))/>
                <button class="btn primary" type="submit" disabled=move || busy.get()><Icon name="search" size=16/>"Search"</button>
            </form>
            <label class="check">
                <input type="checkbox" prop:checked=move || search_now.get()
                    on:change=move |ev| search_now.set(event_target_checked(&ev))/>
                "Search for a release as soon as it is added"
            </label>
            <Notice msg=note/>
            {move || results.get().map(|list| {
                if list.is_empty() {
                    return view! { <p class="weak">"Nothing found."</p> }.into_any();
                }
                let add = add.clone();
                view! {
                    <div class="list panel">
                        {list.into_iter().map(|c| {
                            let add = add.clone();
                            let img = api::poster_url(&c.poster_path, "w92");
                            let id = c.tmdb_id;
                            let year = c.year.map(|y| format!(" ({y})")).unwrap_or_default();
                            view! {
                                <div class="item">
                                    <div style="width:46px;flex:0 0 46px" class="poster">
                                        {img.map(|src| view! { <img src=src alt="" loading="lazy"/> })}
                                    </div>
                                    <div class="grow col" style="gap:2px">
                                        <span style="font-weight:600">{c.title.clone()}{year}</span>
                                        <span class="small weak" style="display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden">{c.overview.clone()}</span>
                                    </div>
                                    {match c.library_id {
                                        Some(lid) => view! { <A href=format!("/movies/{lid}") attr:class="btn small">"In library"</A> }.into_any(),
                                        None => view! { <button class="btn small primary" on:click=move |_| add(id)>"Add"</button> }.into_any(),
                                    }}
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            })}
        </div>
    }
}

// ---------------------------------------------------------------- movie detail

#[component]
pub fn MovieDetailPage() -> impl IntoView {
    let params = use_params_map();
    let id = move || params.with(|p| p.get("id").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0));
    let detail = LocalResource::new(move || {
        let id = id();
        async move { api::get::<MovieDetail>(&format!("/movies/{id}")).await }
    });
    every(10, move || detail.refetch());
    let note = msg();
    let releases: RwSignal<Option<Vec<Release>>> = RwSignal::new(None);
    let busy = RwSignal::new(false);

    let auto = move |_| {
        busy.set(true);
        note.set(Some((true, "Searching indexers…".into())));
        leptos::task::spawn_local(async move {
            match api::post::<_, Option<Release>>(&format!("/movies/{}/search", id()), &()).await {
                Ok(Some(r)) => note.set(Some((true, format!("Grabbed {}", r.title)))),
                Ok(None) => note.set(Some((
                    false,
                    "No acceptable release found. Try the interactive search to see why.".into(),
                ))),
                Err(e) => note.set(Some((false, e.message))),
            }
            busy.set(false);
            detail.refetch();
        });
    };
    let interactive = move |_| {
        busy.set(true);
        note.set(Some((true, "Searching indexers…".into())));
        leptos::task::spawn_local(async move {
            match api::get::<Vec<Release>>(&format!("/movies/{}/releases", id())).await {
                Ok(r) => {
                    note.set(Some((true, format!("{} releases found", r.len()))));
                    releases.set(Some(r));
                }
                Err(e) => note.set(Some((false, e.message))),
            }
            busy.set(false);
        });
    };
    let grab = move |r: Release| {
        leptos::task::spawn_local(async move {
            let g = Grab {
                movie_id: id(),
                release: r.clone(),
            };
            match api::post_unit("/grab", &g).await {
                Ok(()) => {
                    note.set(Some((
                        true,
                        format!("Sent {} to the download client", r.title),
                    )));
                    releases.set(None);
                    detail.refetch();
                }
                Err(e) => note.set(Some((false, e.message))),
            }
        });
    };
    let delete_file = RwSignal::new(false);
    let confirm_delete = RwSignal::new(false);
    let remove = move |_| {
        leptos::task::spawn_local(async move {
            let q = if delete_file.get_untracked() {
                "?delete_file=true"
            } else {
                ""
            };
            match api::delete(&format!("/movies/{}{q}", id())).await {
                Ok(()) => {
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().set_href("/");
                    }
                }
                Err(e) => note.set(Some((false, e.message))),
            }
        });
    };
    let unblock = move |bid: i64| {
        leptos::task::spawn_local(async move {
            if let Err(e) = api::delete(&format!("/blocklist/{bid}")).await {
                note.set(Some((false, e.message)));
            }
            detail.refetch();
        });
    };

    view! {
        <div class="page">
            {move || match detail.get() {
                None => view! { <p class="weak">"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <div class="notice error">{e.message}</div> }.into_any(),
                Some(Ok(d)) => {
                    let m = d.movie.clone();
                    let img = api::poster_url(&m.poster_path, "w500");
                    let chip = format!("chip {}", m.status.as_str());
                    view! {
                        <div class="detail">
                            <div class="poster">
                                {img.map(|src| view! { <img src=src alt=""/> })}
                                <span class=chip>{ui::status_label(m.status)}</span>
                            </div>
                            <div class="info">
                                <h1>{m.title.clone()}</h1>
                                <div class="facts">
                                    {m.year.map(|y| view! { <span>{y}</span> })}
                                    {m.original_title.clone().map(|o| view! { <span>{o}</span> })}
                                    <span>"TMDB "<Copyable text=m.tmdb_id.to_string()/></span>
                                    {m.imdb_id.clone().map(|i| view! { <span>"IMDb "<Copyable text=i/></span> })}
                                </div>
                                <p>{m.overview.clone()}</p>
                                {m.file_path.clone().map(|f| view! {
                                    <div class="col" style="gap:4px">
                                        <span class="small weak">"File"
                                            {m.file_quality.map(|q| format!(" · {}", q.label()))}
                                        </span>
                                        <Copyable text=f/>
                                    </div>
                                })}
                                <div class="row">
                                    <button class="btn primary" on:click=auto disabled=move || busy.get()><Icon name="search" size=16/>"Search now"</button>
                                    <button class="btn" on:click=interactive disabled=move || busy.get()>"Interactive search"</button>
                                    <button class="btn danger" on:click=move |_| confirm_delete.set(true)><Icon name="trash" size=16/>"Remove"</button>
                                </div>
                                {move || confirm_delete.get().then(|| view! {
                                    <div class="panel">
                                        <p>"Remove this movie from Heistarr?"</p>
                                        <label class="check">
                                            <input type="checkbox" prop:checked=move || delete_file.get()
                                                on:change=move |ev| delete_file.set(event_target_checked(&ev))/>
                                            "Also delete the file from disk"
                                        </label>
                                        <div class="row">
                                            <button class="btn danger" on:click=remove>"Remove"</button>
                                            <button class="btn" on:click=move |_| confirm_delete.set(false)>"Cancel"</button>
                                        </div>
                                    </div>
                                })}
                            </div>
                        </div>
                        <Notice msg=note/>
                        {move || releases.get().map(|list| view! { <ReleaseList list=list grab=grab/> })}
                        {(!d.downloads.is_empty()).then(|| view! {
                            <section class="panel">
                                <h2>"Downloads"</h2>
                                <div class="list">
                                    {d.downloads.clone().into_iter().map(|x| view! {
                                        <div class="item">
                                            <span class="grow mono small">{x.release_title}</span>
                                            <span class="kind">{x.state}</span>
                                            <span class="small weak">{ui::time(x.added_at)}</span>
                                        </div>
                                    }).collect_view()}
                                </div>
                            </section>
                        })}
                        {(!d.blocklist.is_empty()).then(|| view! {
                            <section class="panel">
                                <h2>"Blocklist"</h2>
                                <p class="hint">"These releases failed or were removed, and are never grabbed again for this movie."</p>
                                <div class="list">
                                    {d.blocklist.clone().into_iter().map(|b| {
                                        let bid = b.id;
                                        view! {
                                            <div class="item">
                                                <span class="grow mono small">{b.release_title}</span>
                                                <span class="small weak">{b.reason}</span>
                                                <button class="btn small" on:click=move |_| unblock(bid)>"Unblock"</button>
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            </section>
                        })}
                    }.into_any()
                }
            }}
        </div>
    }
}

#[component]
fn ReleaseList(
    list: Vec<Release>,
    grab: impl Fn(Release) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    if list.is_empty() {
        return view! { <div class="panel"><p class="weak">"No releases found on your indexers."</p></div> }.into_any();
    }
    let best_title = list
        .iter()
        .find(|r| r.score.is_some())
        .map(|r| r.title.clone());
    view! {
        <section class="panel">
            <h2>"Releases"</h2>
            <div class="releases">
                {list.into_iter().map(|r| {
                    let best = best_title.as_deref() == Some(r.title.as_str());
                    let rejected = r.score.is_none();
                    let class = if best { "release best" } else if rejected { "release rejected" } else { "release" };
                    let rr = r.clone();
                    view! {
                        <div class=class>
                            <div class="row spread">
                                <span class="name">{r.title.clone()}</span>
                                {best.then(|| view! { <span class="small mono" style="color:var(--accent);display:inline-flex;gap:4px;align-items:center"><Icon name="star" size=13/>"BEST"</span> })}
                            </div>
                            <div class="meta">
                                <span>{r.quality.clone()}</span>
                                <span>{ui::size(r.size)}</span>
                                {r.seeders.map(|s| view! { <span>{format!("{s} seeders")}</span> })}
                                <span>{r.indexer.clone()}</span>
                                <span class="score">{r.score.map(|s| s.to_string()).unwrap_or_else(|| "—".into())}</span>
                                <button class="btn small" title=if rejected { "Grab it anyway" } else { "Send to the download client" }
                                    on:click=move |_| grab(rr.clone())>"Grab"</button>
                            </div>
                            {(!r.rejections.is_empty()).then(|| view! {
                                <div class="why">"Rejected: "{r.rejections.join("; ")}</div>
                            })}
                        </div>
                    }
                }).collect_view()}
            </div>
        </section>
    }
    .into_any()
}

// ---------------------------------------------------------------- downloads

#[component]
pub fn Downloads() -> impl IntoView {
    let downloads = LocalResource::new(|| api::get::<Vec<Download>>("/downloads"));
    let note = msg();
    every(5, move || downloads.refetch());
    let remove = move |id: i64, blocklist: bool| {
        leptos::task::spawn_local(async move {
            #[derive(serde::Serialize)]
            struct Body {
                blocklist: bool,
            }
            match api::post_unit(&format!("/downloads/{id}/remove"), &Body { blocklist }).await {
                Ok(()) => downloads.refetch(),
                Err(e) => note.set(Some((false, e.message))),
            }
        });
    };
    view! {
        <div class="page">
            <h1>"Downloads"</h1>
            <Notice msg=note/>
            <section class="panel">
                {move || match downloads.get() {
                    None => view! { <p class="weak">"Loading…"</p> }.into_any(),
                    Some(Err(e)) => view! { <div class="notice error">{e.message}</div> }.into_any(),
                    Some(Ok(list)) if list.is_empty() => view! { <p class="weak">"Nothing downloading."</p> }.into_any(),
                    Some(Ok(list)) => view! {
                        <div class="list">
                            {list.into_iter().map(|d| {
                                let pct = (d.progress * 100.0).round() as u32;
                                let id = d.id;
                                view! {
                                    <div class="item">
                                        <div class="grow col" style="gap:3px">
                                            <A href=format!("/movies/{}", d.movie_id)><b>{d.movie_title}</b></A>
                                            <span class="mono small weak ellipsis">{d.release_title}</span>
                                        </div>
                                        <div class="row" style="flex:2 1 220px">
                                            <div class="bar"><div style=format!("width:{pct}%")></div></div>
                                            <span class="mono small" style="min-width:40px;text-align:right">{format!("{pct}%")}</span>
                                        </div>
                                        <span class="kind">{d.state}</span>
                                        <div class="row">
                                            <button class="btn small" on:click=move |_| remove(id, false)>"Remove"</button>
                                            <button class="btn small danger" title="Remove and never grab this release again"
                                                on:click=move |_| remove(id, true)>"Remove + block"</button>
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any(),
                }}
            </section>
        </div>
    }
}

// ---------------------------------------------------------------- history

#[component]
pub fn History() -> impl IntoView {
    let events = LocalResource::new(|| api::get::<Vec<HistoryEvent>>("/history"));
    view! {
        <div class="page">
            <div class="row spread">
                <h1>"History"</h1>
                <button class="btn" on:click=move |_| events.refetch()>"Refresh"</button>
            </div>
            <section class="panel">
                {move || match events.get() {
                    None => view! { <p class="weak">"Loading…"</p> }.into_any(),
                    Some(Err(e)) => view! { <div class="notice error">{e.message}</div> }.into_any(),
                    Some(Ok(list)) if list.is_empty() => view! { <p class="weak">"Nothing yet."</p> }.into_any(),
                    Some(Ok(list)) => view! {
                        <div class="list">
                            {list.into_iter().map(|e| {
                                let class = format!("kind {}", e.kind);
                                view! {
                                    <div class="item">
                                        <span class="mono small weak" style="min-width:130px">{ui::time(e.at)}</span>
                                        <span class=class>{e.kind}</span>
                                        <span class="grow">{e.message}</span>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any(),
                }}
            </section>
        </div>
    }
}

// ---------------------------------------------------------------- about

#[component]
pub fn About() -> impl IntoView {
    view! {
        <div class="page">
            <h1>"About"</h1>
            <section class="panel">
                <h2>{concat!("Heistarr ", env!("CARGO_PKG_VERSION"))}</h2>
                <p>"A self-hosted media manager: find movies, pick the best release with your rules, send it to your download client and file it in your library."</p>
            </section>
            <section class="panel">
                <h2>"The Movie Database (TMDB)"</h2>
                <p>"This product uses the TMDB API but is not endorsed or certified by TMDB."</p>
                <p>"Movie titles, overviews and posters come from "
                    <a href="https://www.themoviedb.org/" target="_blank" rel="noopener noreferrer">"themoviedb.org"</a>
                    ", using your own API key."</p>
            </section>
        </div>
    }
}

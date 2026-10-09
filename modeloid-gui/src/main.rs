use dioxus::prelude::*;

mod api;
mod components;
#[cfg(feature = "server")]
mod launcher;
#[cfg(feature = "server")]
mod main_thread;
mod nav;
mod pages;
mod routes;
mod state;
mod util;

use api::appearance::get_appearance;
use components::Fonts;
use routes::Route;
use state::AppearanceState;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");
const THEMES_CSS: Asset = asset!("/assets/themes.css");

fn main() {
    // Outside `dx serve`: one copy per user, on a free port; a second launch
    // only opens the browser on the running one.
    #[cfg(feature = "server")]
    if launcher::prepare() == launcher::Launch::AlreadyRunning {
        return;
    }

    // Threads, connections and the other system settings saved on this computer.
    #[cfg(feature = "server")]
    modeloid::Settings::load_default().apply_system();

    // The server adds plain file routes (dataset images) next to the app, and
    // refuses requests from other sites (`api::guard`).
    // On macOS it runs off the main thread, which native dialogs need.
    #[cfg(feature = "server")]
    main_thread::serve(|| {
        dioxus::serve(|| async move {
            Ok(dioxus::server::router(App)
                .merge(api::files::routes())
                .merge(launcher::routes())
                .layer(dioxus::server::axum::middleware::from_fn(api::guard::local_only)))
        })
    });

    #[cfg(not(feature = "server"))]
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    // Read while server rendering, so the first paint has the saved look.
    let saved = use_server_future(get_appearance)?;
    let appearance = use_context_provider(|| {
        let saved = saved.peek().as_ref().and_then(|r| r.as_ref().ok()).cloned();
        AppearanceState(Signal::new(saved.unwrap_or_default()))
    })
    .0;

    // The wrapper below themes the app; <html> follows too, for the page
    // background, scrollbars and `color-scheme`.
    use_effect(move || {
        let a = appearance.read();
        document::eval(&format!(
            "const d = document.documentElement.dataset; d.theme = {:?}; d.font = {:?}; d.scale = {:?};",
            a.theme, a.font, a.scale
        ));
    });

    let a = appearance.read();
    rsx! {
        document::Title { "Modeloid" }
        document::Link { rel: "icon", href: FAVICON }
        Fonts { active: a.font.clone() }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: THEMES_CSS }
        div {
            class: "app-theme",
            "data-theme": "{a.theme}",
            "data-font": "{a.font}",
            "data-scale": "{a.scale}",
            "data-icons": if a.hide_icons { "off" } else { "on" },
            Router::<Route> {}
        }
    }
}
